//! Quais apps aparecem no dock: os fixados (salvos num arquivo de texto seu)
//! e os que têm janela aberta. Tudo lido do próprio Windows, nada sai do PC.

use crate::mola::Mola;
use std::path::{Path, PathBuf};
use std::time::Instant;
use windows::core::{w, Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Item especial do dock que abre o Launchpad do Cais.
pub const LAUNCHPAD: &str = "cais:launchpad";
/// Item especial do dock que abre o Arquivos do Cais (o nome interno
/// continua "finder" para não mexer nas configurações de quem já usa).
pub const FINDER: &str = "cais:finder";
/// Lixeira no fim do dock.
pub const LIXEIRA: &str = "cais:lixeira";

/// Itens do dock que são do próprio Cais (desenhados à mão).
pub fn eh_especial(c: &str) -> bool {
    c == LAUNCHPAD || c == FINDER || c == LIXEIRA
}

pub struct Item {
    /// Caminho do executável em minúsculas ("" se não deu para descobrir).
    pub exe: String,
    /// O que abrir ao clicar: atalho .lnk ou o próprio .exe.
    pub caminho: String,
    pub nome: String,
    pub fixado: bool,
    /// Janelas abertas, da mais à frente para a mais atrás.
    pub janelas: Vec<HWND>,
    pub icone: Option<ID2D1Bitmap1>,
    pub mola: Mola,
    /// Deslize horizontal (DIPs) quando o item troca de lugar ao arrastar.
    pub desloc: Mola,
    pub pulo: Option<Instant>,
    /// App rodando por um "hospedeiro" (Electron, Java...): o ícone vem
    /// desta janela, não do .exe.
    pub janela_icone: Option<HWND>,
}

impl Item {
    fn novo(caminho: String, exe: String, nome: String, fixado: bool) -> Self {
        Self {
            exe,
            caminho,
            nome,
            fixado,
            janelas: Vec::new(),
            icone: None,
            mola: Mola::default(),
            desloc: Mola::default(),
            pulo: None,
            janela_icone: None,
        }
    }

    /// Pasta no dock (abre como pilha).
    pub fn eh_pasta(&self) -> bool {
        self.exe.is_empty() && !eh_especial(&self.caminho) && Path::new(&self.caminho).is_dir()
    }

    /// Fica no grupo da direita do dock (pastas e lixeira, como no Mac).
    pub fn na_direita(&self) -> bool {
        self.fixado && (self.caminho == LIXEIRA || self.eh_pasta())
    }

    fn de_caminho(c: &str) -> Self {
        use crate::idioma::t;
        if c == LAUNCHPAD {
            return Item::novo(c.into(), String::new(), t("Apps", "Apps").into(), true);
        }
        if c == FINDER {
            return Item::novo(c.into(), String::new(), t("Arquivos", "Files").into(), true);
        }
        if c == LIXEIRA {
            return Item::novo(c.into(), String::new(), t("Lixeira", "Trash").into(), true);
        }
        if Path::new(c).is_dir() {
            let nome = Path::new(c).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| c.to_string());
            return Item::novo(c.into(), String::new(), nome, true);
        }
        if let Some(aumid) = aumid_de_pasta_de_apps(c) {
            let nome = nome_de_app_instalado(c).unwrap_or_else(|| aumid.to_string());
            return Item::novo(c.into(), chave_de_aumid(aumid), nome, true);
        }
        if c.to_lowercase().ends_with(".lnk") {
            // Atalho de app instalado pelo navegador (PWA): agrupa pela
            // identidade do app, não pelo chrome_proxy.exe.
            if let Some(aumid) = aumid_do_atalho(c).filter(|a| eh_aumid_de_pwa(a)) {
                let nome = Path::new(c).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                return Item::novo(c.into(), chave_de_aumid(&aumid), nome, true);
            }
            let exe = resolver_atalho(c).unwrap_or_default().to_lowercase();
            let nome = Path::new(c)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            Item::novo(c.into(), exe, nome, true)
        } else {
            Item::novo(c.into(), c.to_lowercase(), nome_do_exe(c), true)
        }
    }
}

pub struct Modelo {
    pub itens: Vec<Item>,
    arquivo: PathBuf,
}

impl Modelo {
    pub fn carregar() -> Self {
        let dir = PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join("Cais");
        let _ = std::fs::create_dir_all(&dir);
        let arquivo = dir.join("fixados.txt");
        let caminhos: Vec<String> = match std::fs::read_to_string(&arquivo) {
            Ok(t) => t
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect(),
            Err(_) => importar_da_barra_de_tarefas(),
        };
        let mut m = Modelo {
            itens: Vec::new(),
            arquivo,
        };
        for c in caminhos {
            if c.to_lowercase().ends_with(".lnk") && eh_atalho_do_explorador(&c) {
                continue;
            }
            let it = Item::de_caminho(&c);
            let repetido = !it.exe.is_empty() && m.itens.iter().any(|i| i.exe == it.exe);
            if !repetido {
                m.itens.push(it);
            }
        }
        m.ordenar_direita();
        m.salvar();
        m.atualizar_janelas();
        m
    }

    /// Pastas e lixeira vão para o fim (o grupo da direita).
    fn ordenar_direita(&mut self) {
        let (dir, resto): (Vec<Item>, Vec<Item>) = std::mem::take(&mut self.itens).into_iter().partition(|i| i.na_direita());
        self.itens = resto;
        // Lixeira sempre por último.
        let (lixo, pastas): (Vec<Item>, Vec<Item>) = dir.into_iter().partition(|i| i.caminho == LIXEIRA);
        self.itens.extend(pastas);
        self.itens.extend(lixo);
    }

    /// Idioma mudou: os nomes dos itens do próprio Cais acompanham.
    pub fn renomear_especiais(&mut self) {
        for it in &mut self.itens {
            if eh_especial(&it.caminho) {
                it.nome = Item::de_caminho(&it.caminho).nome;
            }
        }
    }

    /// Quantos itens estão no grupo da direita.
    pub fn qtd_direita(&self) -> usize {
        self.itens.iter().filter(|i| i.na_direita()).count()
    }

    /// Põe Downloads e a Lixeira no fim do dock (uma vez só).
    pub fn adicionar_pilhas(&mut self) {
        if let Some(d) = pasta_conhecida(&windows::Win32::UI::Shell::FOLDERID_Downloads) {
            let c = d.to_string_lossy().into_owned();
            if !self.itens.iter().any(|i| i.caminho.eq_ignore_ascii_case(&c)) {
                self.itens.push(Item::de_caminho(&c));
            }
        }
        if !self.itens.iter().any(|i| i.caminho == LIXEIRA) {
            self.itens.push(Item::de_caminho(LIXEIRA));
        }
        self.ordenar_direita();
        self.salvar();
    }

    pub fn salvar(&self) {
        let linhas: Vec<&str> = self
            .itens
            .iter()
            .filter(|i| i.fixado)
            .map(|i| i.caminho.as_str())
            .collect();
        let _ = std::fs::write(&self.arquivo, linhas.join("\r\n"));
    }

    /// Coloca o Launchpad no dock (uma vez só: se você tirar, ele não volta).
    pub fn adicionar_launchpad(&mut self) {
        if self.itens.iter().any(|i| i.caminho == LAUNCHPAD) {
            return;
        }
        let pos = 1.min(self.qtd_fixados());
        self.itens.insert(pos, Item::de_caminho(LAUNCHPAD));
        self.salvar();
    }

    /// Coloca o Finder no começo do dock (uma vez só).
    pub fn adicionar_finder(&mut self) {
        if self.itens.iter().any(|i| i.caminho == FINDER) {
            return;
        }
        self.itens.insert(0, Item::de_caminho(FINDER));
        self.salvar();
    }

    /// Fixados do grupo da esquerda (sem pastas e lixeira).
    pub fn qtd_fixados(&self) -> usize {
        self.itens.iter().filter(|i| i.fixado && !i.na_direita()).count()
    }

    /// Relê as janelas abertas. Mantém a ordem de quem já estava no dock.
    pub fn atualizar_janelas(&mut self) {
        let janelas = enumerar_janelas();
        for it in &mut self.itens {
            it.janelas.clear();
        }
        let tem_por_aumid = self.itens.iter().any(|i| i.exe.starts_with(PREFIXO_AUMID));
        for (h, exe) in janelas {
            let mut pos = self.itens.iter().position(|i| !i.exe.is_empty() && i.exe == exe);
            if pos.is_none() && tem_por_aumid && !exe.starts_with(PREFIXO_AUMID) {
                // App da Loja fixado pela pasta de apps: casa pela identidade
                // (da janela, ou do pacote do processo, como no Paint novo).
                if let Some(a) = aumid_da_janela(h).or_else(|| aumid_do_processo(pid_do_app(h))) {
                    let chave = chave_de_aumid(&a);
                    pos = self.itens.iter().position(|i| i.exe == chave);
                }
            }
            match pos.map(|p| &mut self.itens[p]) {
                Some(i) => i.janelas.push(h),
                None if exe.starts_with(PREFIXO_AUMID) => {
                    // App instalado pelo navegador: abre e mostra pelo
                    // atalho dele na pasta de apps do Windows.
                    let aumid = aumid_da_janela(h).unwrap_or_default();
                    let caminho = format!(r"shell:AppsFolder\{aumid}");
                    let nome = nome_de_app_instalado(&caminho).unwrap_or_else(|| titulo_da_janela(h));
                    let mut it = Item::novo(caminho, exe.clone(), nome, false);
                    it.janelas.push(h);
                    self.itens.push(it);
                }
                None if eh_hospedeiro(&exe) => {
                    let nome = nome_pelo_titulo(h).unwrap_or_else(|| nome_do_exe(&exe));
                    let mut it = Item::novo(exe.clone(), exe.clone(), nome, false);
                    it.janela_icone = Some(h);
                    it.janelas.push(h);
                    self.itens.push(it);
                }
                None => {
                    let mut it = Item::novo(exe.clone(), exe.clone(), nome_do_exe(&exe), false);
                    it.janelas.push(h);
                    self.itens.push(it);
                }
            }
        }
        self.itens.retain(|i| i.fixado || !i.janelas.is_empty());
        // Apps abertos entram antes do grupo da direita (pastas e lixeira).
        if self.qtd_direita() > 0 && self.itens.last().is_some_and(|i| !i.na_direita()) {
            self.ordenar_direita();
        }
    }

    pub fn fixar(&mut self, i: usize) {
        let mut it = self.itens.remove(i);
        it.fixado = true;
        let pos = self.qtd_fixados();
        self.itens.insert(pos, it);
        self.salvar();
    }

    pub fn desafixar(&mut self, i: usize) {
        let mut it = self.itens.remove(i);
        it.fixado = false;
        if !it.janelas.is_empty() {
            let pos = self.qtd_fixados();
            self.itens.insert(pos, it);
        }
        self.salvar();
    }

    pub fn carregar_icones(&mut self, dc: &ID2D1DeviceContext, px: u32) {
        for it in self
            .itens
            .iter_mut()
            .filter(|i| i.icone.is_none() && !eh_especial(&i.caminho))
        {
            it.icone = match it.janela_icone {
                Some(h) => icone_da_janela(dc, h, px).or_else(|| carregar_icone(dc, &it.caminho, px)),
                None => carregar_icone(dc, &it.caminho, px),
            };
        }
    }

    /// Os ícones vão ser carregados de novo (ex.: ligou os ícones uniformes).
    pub fn esquecer_icones(&mut self) {
        for it in &mut self.itens {
            it.icone = None;
        }
    }
}

fn largo(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Primeira execução: Explorador + o que estiver fixado na barra de tarefas.
fn importar_da_barra_de_tarefas() -> Vec<String> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let mut v = vec![format!(r"{windir}\explorer.exe")];
    let pasta = PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
        .join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar");
    if let Ok(rd) = std::fs::read_dir(pasta) {
        let mut atalhos: Vec<String> = rd
            .flatten()
            .map(|e| e.path().to_string_lossy().into_owned())
            .filter(|p| p.to_lowercase().ends_with(".lnk"))
            .collect();
        atalhos.sort_by_key(|s| s.to_lowercase());
        for a in atalhos {
            if !eh_atalho_do_explorador(&a) {
                v.push(a);
            }
        }
    }
    v
}

/// O atalho "File Explorer" da barra aponta para uma pasta virtual, não
/// para o explorer.exe; o Explorador já entra como primeiro item.
fn eh_atalho_do_explorador(lnk: &str) -> bool {
    let exe = resolver_atalho(lnk).unwrap_or_default().to_lowercase();
    if exe.ends_with(r"\explorer.exe") {
        return true;
    }
    let nome = Path::new(lnk)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    exe.is_empty() && (nome == "file explorer" || nome == "explorador de arquivos")
}

fn resolver_atalho(lnk: &str) -> Option<String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let arquivo: IPersistFile = link.cast().ok()?;
        let w = largo(lnk);
        arquivo.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        if n == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

pub fn nome_do_exe(exe: &str) -> String {
    if exe.to_lowercase().ends_with(r"\explorer.exe") {
        return crate::idioma::t("Explorador de Arquivos", "File Explorer").into();
    }
    descricao_do_arquivo(exe).unwrap_or_else(|| {
        Path::new(exe)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

/// "Descrição do arquivo" das propriedades do .exe (ex.: "Google Chrome").
fn descricao_do_arquivo(exe: &str) -> Option<String> {
    unsafe {
        let w = largo(exe);
        let tam = GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None);
        if tam == 0 {
            return None;
        }
        let mut buf = vec![0u8; tam as usize];
        GetFileVersionInfoW(PCWSTR(w.as_ptr()), 0, tam, buf.as_mut_ptr() as _).ok()?;
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut n = 0u32;
        if !VerQueryValueW(
            buf.as_ptr() as _,
            w!("\\VarFileInfo\\Translation"),
            &mut p,
            &mut n,
        )
        .as_bool()
            || n < 4
        {
            return None;
        }
        let idioma = *(p as *const u16);
        let pagina = *(p as *const u16).add(1);
        let sub = largo(&format!(
            "\\StringFileInfo\\{idioma:04x}{pagina:04x}\\FileDescription"
        ));
        if !VerQueryValueW(buf.as_ptr() as _, PCWSTR(sub.as_ptr()), &mut p, &mut n).as_bool()
            || n == 0
        {
            return None;
        }
        // Alguns .exe (ex.: Spotify) declaram um tamanho maior que o texto:
        // o texto termina no primeiro caractere nulo.
        let s = std::slice::from_raw_parts(p as *const u16, n as usize);
        let fim = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        let texto = String::from_utf16_lossy(&s[..fim]).trim().to_string();
        (!texto.is_empty()).then_some(texto)
    }
}

/// Janelas de "app de verdade", na ordem em que estão empilhadas na tela.
pub fn enumerar_janelas() -> Vec<(HWND, String)> {
    let mut v: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(coletar), LPARAM(&mut v as *mut _ as isize));
    }
    let meu = std::process::id();
    v.into_iter()
        .filter_map(|h| {
            let pid = pid_do_app(h);
            if pid == meu {
                return None;
            }
            let exe = exe_do_pid(pid)?.to_lowercase();
            Some((h, chave_da_janela(h, exe)))
        })
        .collect()
}

/// Chave de agrupamento dos apps instalados pelo navegador (PWA).
pub const PREFIXO_AUMID: &str = "aumid:";

fn chave_de_aumid(aumid: &str) -> String {
    format!("{PREFIXO_AUMID}{}", aumid.to_lowercase())
}

/// Navegadores que hospedam apps instalados (PWA) no mesmo processo.
fn eh_navegador(exe: &str) -> bool {
    ["\\chrome.exe", "\\msedge.exe", "\\brave.exe", "\\vivaldi.exe", "\\opera.exe"].iter().any(|s| exe.ends_with(s))
}

/// Chrome e Edge dão aos apps instalados uma identidade própria
/// ("Chrome._crx_..."), a mesma que a barra de tarefas usa para separar.
fn eh_aumid_de_pwa(aumid: &str) -> bool {
    aumid.to_lowercase().contains("_crx_")
}

/// O que agrupa a janela no dock: o .exe, ou a identidade do app quando é
/// um app instalado pelo navegador.
pub fn chave_da_janela(h: HWND, exe: String) -> String {
    if eh_navegador(&exe) {
        if let Some(a) = aumid_da_janela(h).filter(|a| eh_aumid_de_pwa(a)) {
            return chave_de_aumid(&a);
        }
    }
    exe
}

/// Nome do app instalado pelo navegador dono da janela (None se não for).
pub fn nome_de_pwa(h: HWND, exe: &str) -> Option<String> {
    if !eh_navegador(exe) {
        return None;
    }
    let a = aumid_da_janela(h).filter(|a| eh_aumid_de_pwa(a))?;
    nome_de_app_instalado(&format!(r"shell:AppsFolder\{a}"))
}

/// PKEY_AppUserModel_ID.
const PKEY_AUMID: windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY =
    windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
        pid: 5,
    };

fn aumid_da_janela(h: HWND) -> Option<String> {
    use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
    unsafe {
        let loja: IPropertyStore = SHGetPropertyStoreForWindow(h).ok()?;
        let v = loja.GetValue(&PKEY_AUMID).ok()?.to_string();
        (!v.is_empty()).then_some(v)
    }
}

/// Identidade de um processo empacotado (app da Loja ou Win32 empacotado).
fn aumid_do_processo(pid: u32) -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetApplicationUserModelId(processo: HANDLE, tam: *mut u32, buf: *mut u16) -> i32;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut n = buf.len() as u32;
        let r = GetApplicationUserModelId(h, &mut n, buf.as_mut_ptr());
        let _ = CloseHandle(h);
        if r != 0 || n == 0 {
            return None;
        }
        let fim = buf.iter().position(|&c| c == 0).unwrap_or(n as usize);
        Some(String::from_utf16_lossy(&buf[..fim]))
    }
}

fn aumid_do_atalho(lnk: &str) -> Option<String> {
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let arquivo: IPersistFile = link.cast().ok()?;
        let w = largo(lnk);
        arquivo.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
        let loja: IPropertyStore = link.cast().ok()?;
        let v = loja.GetValue(&PKEY_AUMID).ok()?.to_string();
        (!v.is_empty()).then_some(v)
    }
}

/// "shell:AppsFolder\<id>" -> "<id>".
fn aumid_de_pasta_de_apps(c: &str) -> Option<&str> {
    let p = "shell:appsfolder\\";
    (c.len() > p.len() && c[..p.len()].eq_ignore_ascii_case(p)).then(|| &c[p.len()..])
}

/// Nome do app como aparece no menu Iniciar.
fn nome_de_app_instalado(caminho: &str) -> Option<String> {
    unsafe {
        let w = largo(caminho);
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
        let p = item.GetDisplayName(SIGDN_NORMALDISPLAY).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.filter(|s| !s.is_empty())
    }
}

/// Caminho de uma pasta conhecida do Windows (Downloads, Recentes...).
pub fn pasta_conhecida(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KNOWN_FOLDER_FLAG(0), None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.map(PathBuf::from)
    }
}

/// Programa que abre arquivos com esta extensão (".docx" -> winword.exe).
fn programa_da_extensao(ext: &str) -> Option<String> {
    unsafe {
        let e = largo(ext);
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_EXECUTABLE, PCWSTR(e.as_ptr()), PCWSTR::null(), PWSTR(buf.as_mut_ptr()), &mut n).ok().ok()?;
        let fim = buf.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&buf[..fim]).to_lowercase())
    }
}

/// Arquivos abertos há pouco que este app abre (pela lista "Recentes" do
/// próprio Windows: os atalhos em %APPDATA%\Microsoft\Windows\Recent).
pub fn recentes_do_app(exe: &str, max: usize) -> Vec<PathBuf> {
    let Some(pasta) = pasta_conhecida(&FOLDERID_Recent) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(pasta) else { return Vec::new() };
    let mut atalhos: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    atalhos.sort_by(|a, b| b.0.cmp(&a.0));
    let mut por_ext: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut v = Vec::new();
    // Olha só os mais recentes (resolver atalho custa um pouco).
    for (_, lnk) in atalhos.into_iter().take(80) {
        let Some(alvo) = resolver_atalho(&lnk.to_string_lossy()) else { continue };
        let alvo = PathBuf::from(alvo);
        let Some(ext) = alvo.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())) else { continue };
        let deste = *por_ext.entry(ext.clone()).or_insert_with(|| programa_da_extensao(&ext).is_some_and(|p| p == exe));
        if deste && alvo.exists() && !v.contains(&alvo) {
            v.push(alvo);
            if v.len() >= max {
                break;
            }
        }
    }
    v
}

fn titulo_da_janela(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

unsafe extern "system" fn coletar(h: HWND, lp: LPARAM) -> BOOL {
    let v = &mut *(lp.0 as *mut Vec<HWND>);
    if eh_janela_de_app(h, true) {
        v.push(h);
    }
    TRUE
}

/// Mesmo critério da barra de tarefas: visível, sem dono, não é ferramenta,
/// e não está "escondida" pelo sistema (outra área de trabalho virtual etc.).
pub fn eh_janela_de_app(h: HWND, exigir_titulo: bool) -> bool {
    unsafe {
        if !IsWindowVisible(h).as_bool() || h == GetShellWindow() {
            return false;
        }
        let estilo = GetWindowLongW(h, GWL_STYLE) as u32;
        if estilo & WS_CHILD.0 != 0 {
            return false;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        let app = ex & WS_EX_APPWINDOW.0 != 0;
        let tem_dono = GetWindow(h, GW_OWNER)
            .map(|o| !o.0.is_null())
            .unwrap_or(false);
        if !app
            && (ex & WS_EX_TOOLWINDOW.0 != 0 || ex & WS_EX_NOACTIVATE.0 != 0 || tem_dono)
        {
            return false;
        }
        let mut oculta = 0u32;
        let _ = DwmGetWindowAttribute(
            h,
            DWMWA_CLOAKED,
            &mut oculta as *mut _ as _,
            std::mem::size_of::<u32>() as u32,
        );
        if oculta != 0 {
            return false;
        }
        !exigir_titulo || GetWindowTextLengthW(h) > 0
    }
}

/// Apps da Loja rodam dentro do ApplicationFrameHost; o processo real é o
/// dono da CoreWindow filha.
pub fn pid_do_app(h: HWND) -> u32 {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let hospedeiro = exe_do_pid(pid)
            .map(|e| e.to_lowercase().ends_with(r"\applicationframehost.exe"))
            .unwrap_or(false);
        if hospedeiro {
            if let Ok(filho) = FindWindowExW(
                h,
                None,
                w!("Windows.UI.Core.CoreWindow"),
                PCWSTR::null(),
            ) {
                let mut p2 = 0u32;
                GetWindowThreadProcessId(filho, Some(&mut p2));
                if p2 != 0 {
                    return p2;
                }
            }
        }
        pid
    }
}

pub fn exe_do_pid(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n);
        let _ = CloseHandle(h);
        r.ok()?;
        Some(String::from_utf16_lossy(&buf[..n as usize]))
    }
}

/// Ícone grande do app. Primeiro procura no cache em disco
/// (%LOCALAPPDATA%\Cais\icones); se não tiver, pede ao Windows e guarda.
pub fn carregar_icone(dc: &ID2D1DeviceContext, caminho: &str, px: u32) -> Option<ID2D1Bitmap1> {
    if eh_especial(caminho) {
        return None; // desenhado à mão
    }
    let p = pixels_do_icone(caminho, px, false)?;
    bitmap_de_pixels(dc, &crate::placa::aplicar(p))
}

/// Ícone ou miniatura de um arquivo qualquer, sem guardar no cache (as
/// pilhas do dock mostram arquivos que mudam o tempo todo).
pub fn icone_sem_cache(dc: &ID2D1DeviceContext, caminho: &str, px: u32) -> Option<ID2D1Bitmap1> {
    let p = icone_do_windows(caminho, px)?;
    bitmap_de_pixels(dc, &p)
}

pub type Pixels = (u32, u32, Vec<u8>);

/// Pixels do ícone: do cache em disco ou (se `so_cache` for falso) do
/// Windows, guardando no cache. Pode rodar fora da thread da tela.
pub fn pixels_do_icone(caminho: &str, px: u32, so_cache: bool) -> Option<Pixels> {
    let arquivo_cache = caminho_do_cache(caminho, px);
    if let Some(p) = arquivo_cache.as_ref().and_then(|c| ler_cache(c)) {
        return Some(p);
    }
    if so_cache {
        return None;
    }
    let r = icone_do_windows(caminho, px)?;
    if let Some(c) = &arquivo_cache {
        gravar_cache(c, &r);
    }
    Some(r)
}

/// Ícone do Windows (HICON) em bitmap da GPU, no tamanho `px`. Ícones
/// antigos sem canal alfa usam a máscara para saber o que é transparente.
pub fn bitmap_de_hicon(dc: &ID2D1DeviceContext, hicon: HICON, px: u32) -> Option<ID2D1Bitmap1> {
    bitmap_de_pixels(dc, &pixels_de_hicon(hicon, px)?)
}

/// Pixels de um HICON no tamanho `px` (BGRA pré-multiplicado).
pub fn pixels_de_hicon(hicon: HICON, px: u32) -> Option<Pixels> {
    unsafe {
        let pinta = |flags: DI_FLAGS| -> Option<Vec<u8>> {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: px as i32,
                    biHeight: -(px as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let tela = GetDC(None);
            let mem = CreateCompatibleDC(tela);
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(mem, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let antigo = SelectObject(mem, dib);
            let _ = DrawIconEx(mem, 0, 0, hicon, px as i32, px as i32, 0, None, flags);
            let _ = GdiFlush();
            let v = std::slice::from_raw_parts(bits as *const u8, (px * px * 4) as usize).to_vec();
            SelectObject(mem, antigo);
            let _ = DeleteObject(dib);
            let _ = DeleteDC(mem);
            ReleaseDC(None, tela);
            Some(v)
        };
        let mut p = pinta(DI_NORMAL)?;
        if p.chunks(4).all(|c| c[3] == 0) {
            // Sem alfa: a máscara diz o que é transparente (branco = fundo).
            let m = pinta(DI_MASK)?;
            for (c, k) in p.chunks_mut(4).zip(m.chunks(4)) {
                if k[0] > 127 {
                    c.copy_from_slice(&[0, 0, 0, 0]);
                } else {
                    c[3] = 255;
                }
            }
        }
        Some((px, px, p))
    }
}

/// Programas "hospedeiros": rodam o app de outro (um app Electron em
/// desenvolvimento roda pelo electron.exe, um app Java pelo javaw.exe...).
/// O ícone e o nome do .exe não dizem qual é o app; os da janela, sim.
pub const HOSPEDEIROS: [&str; 9] = [
    "electron.exe",
    "node.exe",
    "python.exe",
    "pythonw.exe",
    "java.exe",
    "javaw.exe",
    "wscript.exe",
    "mshta.exe",
    "applicationframehost.exe",
];

pub fn eh_hospedeiro(exe: &str) -> bool {
    exe.rsplit('\\').next().is_some_and(|a| HOSPEDEIROS.iter().any(|h| a.eq_ignore_ascii_case(h)))
}

/// Nome do app pelo título da janela ("documento - App": o app vem no fim).
pub fn nome_pelo_titulo(h: HWND) -> Option<String> {
    let titulo = titulo_da_janela(h);
    let nome = titulo.rsplit(" - ").next()?.trim();
    if nome.is_empty() {
        return None;
    }
    Some(if nome.chars().count() > 32 { nome.chars().take(31).collect::<String>() + "…" } else { nome.to_string() })
}

/// Ícone que o app pôs na própria janela (o maior que ela tiver).
pub fn icone_da_janela(dc: &ID2D1DeviceContext, h: HWND, px: u32) -> Option<ID2D1Bitmap1> {
    unsafe {
        let pedir = |tipo: usize| -> isize {
            let mut r = 0usize;
            let _ = SendMessageTimeoutW(h, WM_GETICON, WPARAM(tipo), LPARAM(0), SMTO_ABORTIFHUNG | SMTO_BLOCK, 150, Some(&mut r));
            r as isize
        };
        let mut hicon = pedir(ICON_BIG as usize);
        if hicon == 0 {
            hicon = GetClassLongPtrW(h, GCLP_HICON) as isize;
        }
        if hicon == 0 {
            hicon = pedir(ICON_SMALL2 as usize);
        }
        if hicon == 0 {
            return None;
        }
        let p = pixels_de_hicon(HICON(hicon as *mut _), px)?;
        bitmap_de_pixels(dc, &crate::placa::aplicar(p))
    }
}

pub fn bitmap_de_pixels(dc: &ID2D1DeviceContext, (l, a, pixels): &Pixels) -> Option<ID2D1Bitmap1> {
    let (l, a) = (*l, *a);
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        ..Default::default()
    };
    unsafe {
        dc.CreateBitmap(
            D2D_SIZE_U {
                width: l,
                height: a,
            },
            Some(pixels.as_ptr() as _),
            l * 4,
            &props,
        )
        .ok()
    }
}

/// O nome do arquivo de cache inclui a data de modificação do app: se o app
/// for atualizado (e o ícone mudar), o cache antigo é ignorado.
fn caminho_do_cache(caminho: &str, px: u32) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    // Apps da pasta Aplicativos ("shell:AppsFolder\...") não são arquivos:
    // a chave fica só com o nome e o tamanho.
    let modificado = std::fs::metadata(caminho).and_then(|m| m.modified()).ok();
    if modificado.is_none() && !caminho.starts_with("shell:") {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    caminho.to_lowercase().hash(&mut h);
    px.hash(&mut h);
    modificado.hash(&mut h);
    let dir = PathBuf::from(std::env::var("LOCALAPPDATA").ok()?).join(r"Cais\icones");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join(format!("{:016x}.bgra", h.finish())))
}

fn ler_cache(c: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let b = std::fs::read(c).ok()?;
    if b.len() < 8 {
        return None;
    }
    let l = u32::from_le_bytes(b[0..4].try_into().ok()?);
    let a = u32::from_le_bytes(b[4..8].try_into().ok()?);
    let px = b[8..].to_vec();
    (px.len() == (l * a * 4) as usize).then_some((l, a, px))
}

fn gravar_cache(c: &Path, (l, a, px): &(u32, u32, Vec<u8>)) {
    let mut b = Vec::with_capacity(8 + px.len());
    b.extend_from_slice(&l.to_le_bytes());
    b.extend_from_slice(&a.to_le_bytes());
    b.extend_from_slice(px);
    let _ = std::fs::write(c, b);
}

/// Pede o ícone ao Windows no tamanho exato (BGRA pré-multiplicado).
fn icone_do_windows(caminho: &str, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let w = largo(caminho);
        let fabrica: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
        let hbm = fabrica
            .GetImage(
                SIZE {
                    cx: px as i32,
                    cy: px as i32,
                },
                SIIGBF_ICONONLY,
            )
            .ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(
            HGDIOBJ(hbm.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as _),
        );
        let (l, a) = (bm.bmWidth, bm.bmHeight.abs());
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: l,
                biHeight: -a,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (l * a * 4) as usize];
        let hdc = GetDC(None);
        let linhas = GetDIBits(
            hdc,
            hbm,
            0,
            a as u32,
            Some(pixels.as_mut_ptr() as _),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, hdc);
        let _ = DeleteObject(HGDIOBJ(hbm.0));
        if linhas == 0 || l == 0 {
            return None;
        }
        // Ícones antigos sem canal alfa: tornar opacos.
        if pixels.chunks(4).all(|p| p[3] == 0) {
            for p in pixels.chunks_mut(4) {
                p[3] = 255;
            }
        }
        Some((l as u32, a as u32, pixels))
    }
}

/// Clique num app: abre, traz para frente, alterna entre janelas ou minimiza.
/// Devolve true se abriu um app novo (para o ícone pular).
pub fn acionar(it: &Item) -> bool {
    unsafe {
        if it.janelas.is_empty() {
            abrir(&it.caminho);
            return true;
        }
        let frente = GetForegroundWindow();
        if it.janelas.contains(&frente) {
            if it.janelas.len() > 1 {
                trazer(*it.janelas.last().unwrap());
            } else {
                let _ = ShowWindow(frente, SW_MINIMIZE);
            }
        } else {
            trazer(it.janelas[0]);
        }
        false
    }
}

pub fn abrir(caminho: &str) {
    if crate::pastas::trocar_pelo_finder(caminho) {
        return;
    }
    let w = largo(caminho);
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Executa um programa com parâmetros (ex.: shutdown.exe /s /t 0).
pub fn executar(arquivo: &str, parametros: &str) {
    let a = largo(arquivo);
    let p = largo(parametros);
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(a.as_ptr()),
            PCWSTR(p.as_ptr()),
            PCWSTR::null(),
            SW_HIDE,
        );
    }
}

/// Processos do app, mesmo sem janela (os que ficam só na bandeja). Apps que
/// rodam dentro de outro programa (Electron, Java...) contam só pelos
/// processos das próprias janelas: senão "electron.exe" pegaria todos.
pub fn processos_do_app(it: &Item) -> Vec<u32> {
    let mut v: Vec<u32> = Vec::new();
    for &h in &it.janelas {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(h, Some(&mut pid));
        }
        if pid != 0 && !v.contains(&pid) {
            v.push(pid);
        }
    }
    let exe = it.exe.to_lowercase();
    // Shells rodam scripts em segundo plano (as bandejas, por exemplo):
    // fechar "o PowerShell" não pode derrubar todos.
    let shell = ["powershell.exe", "pwsh.exe", "cmd.exe", "conhost.exe"].iter().any(|s| exe.ends_with(&format!(r"\{s}")));
    if exe.is_empty() || !exe.ends_with(".exe") || eh_hospedeiro(&exe) || shell || exe.ends_with(r"\explorer.exe") {
        return v;
    }
    let mut pids = vec![0u32; 4096];
    let mut bytes = 0u32;
    unsafe {
        use windows::Win32::System::ProcessStatus::K32EnumProcesses;
        if !K32EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut bytes).as_bool() {
            return v;
        }
    }
    pids.truncate(bytes as usize / 4);
    for pid in pids.into_iter().filter(|&p| p > 4 && p != std::process::id()) {
        if !v.contains(&pid) && exe_do_pid(pid).is_some_and(|e| e.to_lowercase() == exe) {
            v.push(pid);
        }
    }
    v
}

/// "Forçar encerramento", como no Mac: termina os processos na hora. Os de
/// administrador recusam (o Windows não deixa) e ficam como estão.
pub fn forcar_encerramento(pids: &[u32]) {
    use windows::Win32::System::Threading::{TerminateProcess, PROCESS_TERMINATE};
    for &pid in pids {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
            }
        }
    }
}

/// Fechar o app: pede às janelas para fechar (o app pode perguntar se quer
/// salvar). Sem janela nenhuma (app só na bandeja), não há a quem pedir:
/// encerra os processos.
pub fn fechar_app(it: &Item) {
    if it.janelas.is_empty() {
        forcar_encerramento(&processos_do_app(it));
    } else {
        fechar_janelas(it);
    }
}

pub fn fechar_janelas(it: &Item) {
    for &h in &it.janelas {
        unsafe {
            let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

fn trazer(h: HWND) {
    trazer_janela(h);
}

/// Traz a janela para a frente (restaurando se estiver minimizada). Se o
/// Windows recusar o foco (ele só deixa quem recebeu o último clique ou
/// tecla), usa o mesmo caminho do Alt+Tab.
pub fn trazer_janela(h: HWND) {
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        if !SetForegroundWindow(h).as_bool() || GetForegroundWindow() != h {
            SwitchToThisWindow(h, true);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn apps_instalados_pelo_navegador() {
        assert!(eh_aumid_de_pwa("Chrome._crx_agimnaknmldooml.UserData.Profile3"));
        assert!(!eh_aumid_de_pwa("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"));
        assert_eq!(chave_de_aumid("Chrome._crx_ABC"), "aumid:chrome._crx_abc");
        assert!(eh_navegador(r"c:\program files\google\chrome\application\chrome.exe"));
        assert!(!eh_navegador(r"c:\windows\notepad.exe"));
    }

    #[test]
    fn pasta_de_apps() {
        assert_eq!(aumid_de_pasta_de_apps(r"shell:AppsFolder\Microsoft.Paint_8wekyb3d8bbwe!App"), Some("Microsoft.Paint_8wekyb3d8bbwe!App"));
        assert_eq!(aumid_de_pasta_de_apps(r"C:\Windows\notepad.exe"), None);
    }
}
