//! Catálogo de apps instalados (a pasta "Aplicativos" do Windows, a mesma
//! do menu Iniciar), busca por nome, atalhos de Configurações e calculadora.
//!
//! Tudo é local: a lista vem do próprio Windows e o histórico de uso (para
//! os apps que você mais abre subirem na busca) fica em
//! %APPDATA%\Cais\historico.txt. Nada de busca na web.

use crate::apps;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// A thread do catálogo terminou algo. lParam: 0 = lista pronta,
/// 1 = mais ícones prontos no cache.
pub const WM_CATALOGO: u32 = WM_APP + 6;
/// Tamanho dos ícones (DIPs) guardados no cache: serve ao Launchpad e,
/// reduzido, à busca.
pub const ICONE_DIP: f32 = 80.0;

#[derive(Clone)]
pub struct AppInstalado {
    pub nome: String,
    /// O que abrir: "shell:AppsFolder\<id>" (ou ms-settings:...).
    pub abrir: String,
    /// Nome sem acentos, minúsculo (para buscar).
    chave: String,
}

#[derive(Clone, PartialEq)]
pub enum Tipo {
    App,
    Ajuste,
    Conta,
    /// Buscar arquivos com esse nome no Arquivos.
    Arquivos,
    /// Um arquivo achado pelo índice do Windows (valor = caminho).
    Arquivo,
}

#[derive(Clone)]
pub struct Resultado {
    pub titulo: String,
    pub detalhe: String,
    pub tipo: Tipo,
    /// Índice em `apps` (só para Tipo::App).
    pub app: Option<usize>,
    /// O que abrir (Ajuste) ou o texto a copiar (Conta).
    pub valor: String,
}

pub struct Catalogo {
    pub apps: Vec<AppInstalado>,
    pub px: u32,
    carregando: bool,
    pronto: Arc<Mutex<Option<Vec<AppInstalado>>>>,
    icones: HashMap<String, Option<ID2D1Bitmap1>>,
    historico: HashMap<String, u32>,
    arquivo_hist: PathBuf,
    hwnd_aviso: isize,
}

/// (nome em português, nome em inglês, o que abrir). A busca acha pelos dois.
const AJUSTES: &[(&str, &str, &str)] = &[
    ("Wi-Fi", "Wi-Fi", "ms-settings:network-wifi"),
    ("Bluetooth e dispositivos", "Bluetooth & devices", "ms-settings:bluetooth"),
    ("Rede e internet", "Network & internet", "ms-settings:network"),
    ("Som", "Sound", "ms-settings:sound"),
    ("Tela e resolução", "Display & resolution", "ms-settings:display"),
    ("Papel de parede", "Wallpaper", "ms-settings:personalization-background"),
    ("Cores e modo escuro", "Colors & dark mode", "ms-settings:colors"),
    ("Windows Update", "Windows Update", "ms-settings:windowsupdate"),
    ("Energia e bateria", "Power & battery", "ms-settings:powersleep"),
    ("Apps instalados", "Installed apps", "ms-settings:appsfeatures"),
    ("Apps padrão", "Default apps", "ms-settings:defaultapps"),
    ("Inicialização de apps", "Startup apps", "ms-settings:startupapps"),
    ("Privacidade e segurança", "Privacy & security", "ms-settings:privacy"),
    ("Data e hora", "Date & time", "ms-settings:dateandtime"),
    ("Idioma e região", "Language & region", "ms-settings:regionlanguage"),
    ("Mouse", "Mouse", "ms-settings:mousetouchpad"),
    ("Teclado", "Keyboard", "ms-settings:keyboard"),
    ("Impressoras", "Printers", "ms-settings:printers"),
    ("Notificações", "Notifications", "ms-settings:notifications"),
    ("Armazenamento", "Storage", "ms-settings:storagesense"),
    ("Contas", "Accounts", "ms-settings:yourinfo"),
    ("Sobre este computador", "About this PC", "ms-settings:about"),
    ("Painel de Controle", "Control Panel", "control.exe"),
];

impl Catalogo {
    pub fn new(hwnd_aviso: HWND, escala: f32) -> Self {
        let arquivo_hist = PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
            .join(r"Cais\historico.txt");
        let historico = std::fs::read_to_string(&arquivo_hist)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let (n, c) = l.split_once('\t')?;
                Some((c.to_string(), n.trim().parse().ok()?))
            })
            .collect();
        Self {
            apps: Vec::new(),
            px: (ICONE_DIP * escala).round() as u32,
            carregando: false,
            pronto: Arc::new(Mutex::new(None)),
            icones: HashMap::new(),
            historico,
            arquivo_hist,
            hwnd_aviso: hwnd_aviso.0 as isize,
        }
    }

    /// Relê a lista de apps numa thread (a tela não espera). Chamado ao
    /// abrir a busca ou o Launchpad: apps instalados desde a última vez
    /// aparecem sozinhos.
    pub fn atualizar(&mut self) {
        if self.carregando {
            return;
        }
        self.carregando = true;
        let pronto = self.pronto.clone();
        let hwnd = self.hwnd_aviso;
        let px = self.px;
        let _ = std::thread::Builder::new()
            .name("cais-catalogo".into())
            .stack_size(512 * 1024)
            .spawn(move || ler_apps(pronto, hwnd, px));
    }

    /// Mensagem da thread: lista nova (0) ou ícones novos no cache (1).
    pub fn receber(&mut self, tipo: isize) {
        if tipo == 0 {
            if let Some(v) = self.pronto.lock().ok().and_then(|mut p| p.take()) {
                self.apps = v;
            }
        } else if tipo == 2 {
            self.carregando = false;
        }
        // Ícones que faltavam podem estar no cache agora.
        self.icones.retain(|_, b| b.is_some());
    }

    pub fn carregando(&self) -> bool {
        self.carregando && self.apps.is_empty()
    }

    /// Ícone do app (só do cache em disco: quem pede ao Windows é a thread).
    pub fn icone(&mut self, dc: &ID2D1DeviceContext, abrir: &str) -> Option<ID2D1Bitmap1> {
        if let Some(b) = self.icones.get(abrir) {
            return b.clone();
        }
        let b = apps::pixels_do_icone(abrir, self.px, true).and_then(|p| apps::bitmap_de_pixels(dc, &crate::placa::aplicar(p)));
        self.icones.insert(abrir.to_string(), b.clone());
        b
    }

    /// Libera os ícones da memória (busca e Launchpad fechados).
    pub fn soltar_icones(&mut self) {
        self.icones.clear();
        self.icones.shrink_to_fit();
    }

    pub fn usou(&mut self, abrir: &str) {
        *self.historico.entry(abrir.to_string()).or_insert(0) += 1;
        let mut linhas: Vec<(&String, &u32)> = self.historico.iter().collect();
        linhas.sort_by(|a, b| b.1.cmp(a.1));
        let texto: Vec<String> = linhas.iter().take(300).map(|(c, n)| format!("{n}\t{c}")).collect();
        let _ = std::fs::write(&self.arquivo_hist, texto.join("\r\n"));
    }

    pub fn abrir_app(&mut self, i: usize) {
        if let Some(a) = self.apps.get(i) {
            let alvo = a.abrir.clone();
            apps::abrir(&alvo);
            self.usou(&alvo);
        }
    }

    /// Índices dos apps que combinam com o texto, do melhor para o pior.
    pub fn filtrar(&self, consulta: &str) -> Vec<usize> {
        let q = normalizar(consulta.trim());
        if q.is_empty() {
            return (0..self.apps.len()).collect();
        }
        let mut v: Vec<(i32, usize)> = self
            .apps
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                let p = pontuar(&a.chave, &q)?;
                // Quem você abre mais sobe um pouco (sem passar na frente de
                // um nome que combina bem melhor).
                let uso = self.historico.get(&a.abrir).copied().unwrap_or(0).min(15) as i32;
                Some((p + uso * 2, i))
            })
            .collect();
        // Letras soltas no meio do nome só valem se não houver nada melhor.
        if v.iter().any(|(p, _)| *p >= 55) {
            v.retain(|(p, _)| *p >= 55);
        }
        v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| self.apps[a.1].nome.len().cmp(&self.apps[b.1].nome.len())));
        v.into_iter().map(|(_, i)| i).collect()
    }

    /// Resultados da busca: conta (se for uma), apps e ajustes.
    pub fn buscar(&self, consulta: &str, max: usize) -> Vec<Resultado> {
        let mut r = Vec::new();
        if let Some(v) = calcular(consulta) {
            let texto = formatar_numero(v);
            r.push(Resultado {
                titulo: format!("= {texto}"),
                detalhe: crate::idioma::t("Enter copia o resultado", "Enter copies the result").into(),
                tipo: Tipo::Conta,
                app: None,
                valor: texto,
            });
        }
        if consulta.trim().is_empty() {
            return r;
        }
        let q = normalizar(consulta.trim());
        let mut ajustes: Vec<(i32, Resultado)> = AJUSTES
            .iter()
            .filter_map(|(pt, en, abrir)| {
                let p = pontuar(&normalizar(pt), &q).max(pontuar(&normalizar(en), &q))?;
                Some((
                    p,
                    Resultado {
                        titulo: crate::idioma::t(pt, en).to_string(),
                        detalhe: crate::idioma::t("Configurações", "Settings").into(),
                        tipo: Tipo::Ajuste,
                        app: None,
                        valor: abrir.to_string(),
                    },
                ))
            })
            .collect();
        ajustes.sort_by(|a, b| b.0.cmp(&a.0));
        for i in self.filtrar(consulta).into_iter().take(max) {
            r.push(Resultado {
                titulo: self.apps[i].nome.clone(),
                detalhe: "App".into(),
                tipo: Tipo::App,
                app: Some(i),
                valor: self.apps[i].abrir.clone(),
            });
        }
        // Ajustes entram depois dos apps (até 3), a menos que combinem muito.
        let pos_ajuste = r.iter().filter(|x| x.tipo != Tipo::App).count();
        for (k, (p, a)) in ajustes.into_iter().take(3).enumerate() {
            if p >= 90 {
                r.insert(pos_ajuste + k, a);
            } else {
                r.push(a);
            }
        }
        // Por último: procurar arquivos com esse nome (abre o Arquivos).
        r.truncate(max.saturating_sub(1));
        r.push(Resultado {
            titulo: format!("{}: \u{201c}{}\u{201d}", crate::idioma::t("Buscar arquivos", "Search files"), consulta.trim()),
            detalhe: crate::idioma::t("Arquivos", "Files").into(),
            tipo: Tipo::Arquivos,
            app: None,
            valor: consulta.trim().to_string(),
        });
        r
    }
}

// ---------------------------------------------------------------------------
// A thread que lê os apps e esquenta o cache de ícones

fn ler_apps(pronto: Arc<Mutex<Option<Vec<AppInstalado>>>>, hwnd: isize, px: u32) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let avisar = |tipo: isize| unsafe {
        let _ = PostMessageW(HWND(hwnd as *mut _), WM_CATALOGO, WPARAM(0), LPARAM(tipo));
    };
    let mut lista = enumerar().unwrap_or_default();
    lista.sort_by(|a, b| a.chave.cmp(&b.chave));
    lista.dedup_by(|a, b| a.chave == b.chave);
    let caminhos: Vec<String> = lista.iter().map(|a| a.abrir.clone()).collect();
    if let Ok(mut p) = pronto.lock() {
        *p = Some(lista);
    }
    avisar(0);
    // Ícones que ainda não estão no cache: pede ao Windows, em lotes.
    let mut novos = 0;
    for c in &caminhos {
        if apps::pixels_do_icone(c, px, true).is_none() && apps::pixels_do_icone(c, px, false).is_some() {
            novos += 1;
            if novos % 20 == 0 {
                avisar(1);
            }
        }
    }
    if novos > 0 {
        avisar(1);
    }
    avisar(2);
    crate::medidor::enxugar_memoria();
}

fn enumerar() -> Option<Vec<AppInstalado>> {
    unsafe {
        let pasta: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None).ok()?;
        let lista: IEnumShellItems = pasta.BindToHandler(None, &BHID_EnumItems).ok()?;
        let mut v = Vec::new();
        loop {
            let mut um = [None];
            let mut n = 0u32;
            if lista.Next(&mut um, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            let Some(item) = um[0].take() else { break };
            let texto = |s: SIGDN| -> Option<String> {
                let p = item.GetDisplayName(s).ok()?;
                let t = p.to_string().ok();
                CoTaskMemFree(Some(p.0 as _));
                t
            };
            let (Some(nome), Some(id)) = (texto(SIGDN_NORMALDISPLAY), texto(SIGDN_PARENTRELATIVEPARSING)) else {
                continue;
            };
            if !eh_app(&nome, &id) {
                continue;
            }
            v.push(AppInstalado {
                chave: normalizar(&nome),
                abrir: format!("shell:AppsFolder\\{id}"),
                nome,
            });
        }
        Some(v)
    }
}

/// A pasta Aplicativos também lista desinstaladores, manuais e links: fora.
fn eh_app(nome: &str, id: &str) -> bool {
    let n = nome.to_lowercase();
    let i = id.to_lowercase();
    let lixo = ["uninstall", "desinstal", "readme", "leia-me", "release notes", "documentation", "documentação", "help", "ajuda"];
    if lixo.iter().any(|l| n.contains(l)) {
        return false;
    }
    let extensoes = [".url", ".txt", ".chm", ".pdf", ".html", ".htm", ".rtf", ".ini", ".log"];
    !extensoes.iter().any(|e| i.ends_with(e)) && !i.starts_with("http")
}

// ---------------------------------------------------------------------------
// Busca

/// Minúsculas e sem acentos ("Configurações" -> "configuracoes").
pub fn normalizar(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// Quanto o nome combina com a consulta (None = não combina).
fn pontuar(nome: &str, q: &str) -> Option<i32> {
    if nome == q {
        return Some(120);
    }
    if nome.starts_with(q) {
        return Some(100);
    }
    let palavras: Vec<&str> = nome.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()).collect();
    if palavras.iter().any(|p| p.starts_with(q)) {
        return Some(85);
    }
    let iniciais: String = palavras.iter().filter_map(|p| p.chars().next()).collect();
    if q.len() >= 2 && iniciais.starts_with(q) {
        return Some(75); // "vsc" -> Visual Studio Code
    }
    if nome.contains(q) {
        return Some(55);
    }
    // Letras na ordem, com buracos ("gchrm" -> Google Chrome).
    let mut it = nome.chars();
    if q.len() >= 3 && q.chars().all(|c| it.any(|n| n == c)) {
        return Some(25);
    }
    None
}

// ---------------------------------------------------------------------------
// Calculadora

/// Resolve contas simples: + - * / % ^ ( ), com vírgula ou ponto.
/// Só responde se o texto parece uma conta (tem um operador).
pub fn calcular(s: &str) -> Option<f64> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() || !s.chars().any(|c| "+-*/%^x×÷".contains(c)) {
        return None;
    }
    if !s.chars().all(|c| c.is_ascii_digit() || "+-*/%^().,x×÷".contains(c)) {
        return None;
    }
    let toks: Vec<char> = s
        .chars()
        .map(|c| match c {
            ',' => '.',
            'x' | '×' => '*',
            '÷' => '/',
            c => c,
        })
        .collect();
    let mut p = Conta { t: &toks, i: 0 };
    let v = p.soma()?;
    (p.i == toks.len() && v.is_finite()).then_some(v)
}

struct Conta<'a> {
    t: &'a [char],
    i: usize,
}

impl Conta<'_> {
    fn ver(&self) -> Option<char> {
        self.t.get(self.i).copied()
    }
    fn soma(&mut self) -> Option<f64> {
        let mut v = self.produto()?;
        while let Some(c @ ('+' | '-')) = self.ver() {
            self.i += 1;
            let d = self.produto()?;
            v = if c == '+' { v + d } else { v - d };
        }
        Some(v)
    }
    fn produto(&mut self) -> Option<f64> {
        let mut v = self.potencia()?;
        while let Some(c @ ('*' | '/' | '%')) = self.ver() {
            self.i += 1;
            if c == '%' && !matches!(self.ver(), Some('0'..='9' | '(' | '.')) {
                v /= 100.0; // "50%" sozinho = 0,5
                continue;
            }
            let d = self.potencia()?;
            v = match c {
                '*' => v * d,
                '/' => v / d,
                _ => v % d,
            };
        }
        Some(v)
    }
    fn potencia(&mut self) -> Option<f64> {
        let b = self.unario()?;
        if self.ver() == Some('^') {
            self.i += 1;
            let e = self.potencia()?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    fn unario(&mut self) -> Option<f64> {
        match self.ver()? {
            '-' => {
                self.i += 1;
                Some(-self.unario()?)
            }
            '+' => {
                self.i += 1;
                self.unario()
            }
            '(' => {
                self.i += 1;
                let v = self.soma()?;
                if self.ver() == Some(')') {
                    self.i += 1;
                }
                Some(v)
            }
            _ => {
                let ini = self.i;
                while matches!(self.ver(), Some('0'..='9' | '.')) {
                    self.i += 1;
                }
                let n: String = self.t[ini..self.i].iter().collect();
                n.parse().ok()
            }
        }
    }
}

/// 1234.5 -> "1.234,5" (até 10 dígitos significativos).
pub fn formatar_numero(v: f64) -> String {
    if v.abs() >= 1e15 || (v != 0.0 && v.abs() < 1e-9) {
        return format!("{v:e}").replace('.', ",");
    }
    let texto = format!("{:.10}", v);
    let (int, frac) = texto.split_once('.').unwrap_or((&texto, ""));
    let frac = frac.trim_end_matches('0');
    let (sinal, int) = int.strip_prefix('-').map(|i| ("-", i)).unwrap_or(("", int));
    let mut agrupado = String::new();
    for (k, c) in int.chars().enumerate() {
        if k > 0 && (int.len() - k) % 3 == 0 {
            agrupado.push('.');
        }
        agrupado.push(c);
    }
    if frac.is_empty() {
        format!("{sinal}{agrupado}")
    } else {
        format!("{sinal}{agrupado},{frac}")
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn contas() {
        assert_eq!(calcular("2+2"), Some(4.0));
        assert_eq!(calcular("(1440+2560)/4"), Some(1000.0));
        assert_eq!(calcular("12*7"), Some(84.0));
        assert_eq!(calcular("3x4"), Some(12.0));
        assert!(calcular("ola").is_none());
        assert!(calcular("").is_none());
    }

    #[test]
    fn numeros_no_jeito_brasileiro() {
        assert_eq!(formatar_numero(1000.0), "1.000");
        assert_eq!(formatar_numero(2.5), "2,5");
    }

    #[test]
    fn busca_sem_acentos() {
        assert_eq!(normalizar("Configurações"), "configuracoes");
        assert!(pontuar(&normalizar("Calculadora"), &normalizar("calc")).is_some());
        assert!(pontuar(&normalizar("Calculadora"), &normalizar("xyz")).is_none());
    }
}
