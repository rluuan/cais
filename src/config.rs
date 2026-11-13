//! Configurações do Cais num arquivo de texto simples:
//! %APPDATA%\Cais\config.txt (pode editar à mão, ou pela tela de
//! Preferências do Cais).

use std::path::PathBuf;

pub struct Config {
    pub esconder_barra: bool,
    pub ocultar_auto: bool,
    /// Ícones do dock crescem quando o mouse passa.
    pub ampliar: bool,
    /// Para onde vai a janela minimizada: "lateral" (telinhas na esquerda,
    /// como o Stage Manager), "dock" (miniatura no fim do dock) ou "nao".
    pub minimizar_para: String,
    pub lateral_inclinar: bool,
    pub lateral_reservar: bool,
    pub lateral_maximo: usize,
    /// Tamanho das telinhas: "pequeno", "medio" ou "grande".
    pub lateral_tamanho: String,
    /// Efeito gênio ao minimizar para a faixa lateral.
    pub efeito_genio: bool,
    /// Atalho da busca, ex.: "alt+espaco", "ctrl+espaco" ou "nenhum".
    pub atalho_busca: String,
    /// O Launchpad já foi colocado no dock uma vez (se tirar, não volta).
    pub launchpad_no_dock: bool,
    /// O Finder já foi colocado no dock uma vez.
    pub finder_no_dock: bool,
    /// CPU e memória na barra do topo (atualiza a cada 2 s). Padrão: não.
    pub monitor_na_barra: bool,
    /// Layouts de janelas (áreas por monitor). Padrão: não.
    pub layouts: bool,
    /// Índices em layouts::LAYOUTS e layouts::ESPACOS.
    pub layout_principal: usize,
    pub layout_outros: usize,
    pub layout_espaco: usize,
    /// Tema: "automatico", "escuro" ou "claro".
    pub aparencia: String,
    /// Cor de destaque (nomes em tema::DESTAQUES).
    pub destaque: String,
    /// Transparência do vidro: "pouca", "media" ou "muita".
    pub transparencia: String,
    /// Pastas abertas no Explorador passam para o Arquivos do Cais.
    pub pastas_no_finder: bool,
    /// Idioma: "automatico" (segue o Windows), "pt" ou "en".
    pub idioma: String,
    /// Cantos ativos (sup. esq., sup. dir., inf. esq., inf. dir.): valores
    /// de cantos::ACOES ("nada", "visao", "mesa", "apps", "busca", "bloquear").
    pub cantos: [String; 4],
    /// Atalho da Visão Geral (todas as janelas lado a lado).
    pub atalho_visao: String,
    /// Alt+Tab mostra os apps no estilo do Cais (padrão: não).
    pub alternador: bool,
    /// Posição do dock: "baixo", "esquerda" ou "direita".
    pub dock_posicao: String,
    /// Miniaturas das janelas ao parar o mouse num app do dock.
    pub previas_no_dock: bool,
    /// A lixeira já foi colocada no fim do dock uma vez.
    pub lixeira_no_dock: bool,
    /// Animações: "automatico" (segue o Windows), "sim" (reduz) ou "nao".
    pub reduzir_movimento: String,
    /// Menus de apps Electron na barra (pela Automação de Interface).
    pub menus_electron: bool,
    /// Todos os ícones de app na mesma placa arredondada (padrão: sim).
    pub icones_uniformes: bool,
    /// Pede cantos arredondados nas janelas dos outros apps (padrão: não).
    pub arredondar_janelas: bool,
    arquivo: PathBuf,
}

impl Config {
    pub fn carregar() -> Self {
        let arquivo = PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
            .join(r"Cais\config.txt");
        let mut c = Config {
            esconder_barra: true,
            ocultar_auto: false,
            ampliar: true,
            minimizar_para: "lateral".into(),
            lateral_inclinar: true,
            lateral_reservar: true,
            lateral_maximo: 5,
            lateral_tamanho: "grande".into(),
            efeito_genio: true,
            atalho_busca: "alt+espaco".into(),
            launchpad_no_dock: false,
            finder_no_dock: false,
            monitor_na_barra: false,
            layouts: false,
            layout_principal: 2,
            layout_outros: 0,
            layout_espaco: 1,
            aparencia: "automatico".into(),
            destaque: "azul".into(),
            transparencia: "media".into(),
            pastas_no_finder: true,
            idioma: "automatico".into(),
            cantos: ["nada".into(), "nada".into(), "nada".into(), "nada".into()],
            atalho_visao: "win+ctrl+seta_cima".into(),
            alternador: false,
            dock_posicao: "baixo".into(),
            previas_no_dock: true,
            lixeira_no_dock: false,
            reduzir_movimento: "automatico".into(),
            menus_electron: false,
            icones_uniformes: true,
            arredondar_janelas: false,
            arquivo,
        };
        if let Ok(t) = std::fs::read_to_string(&c.arquivo) {
            for linha in t.lines() {
                let Some((k, v)) = linha.split_once('=') else {
                    continue;
                };
                let v = v.trim().to_lowercase();
                let sim = matches!(v.as_str(), "sim" | "s" | "true" | "1");
                match k.trim() {
                    "esconder_barra_de_tarefas" => c.esconder_barra = sim,
                    "ocultar_automaticamente" => c.ocultar_auto = sim,
                    "ampliar_icones" => c.ampliar = sim,
                    "minimizar_para" if matches!(v.as_str(), "lateral" | "dock" | "nao") => c.minimizar_para = v,
                    "lateral_inclinada" => c.lateral_inclinar = sim,
                    "lateral_reservar_espaco" => c.lateral_reservar = sim,
                    "lateral_tamanho" if matches!(v.as_str(), "pequeno" | "medio" | "grande") => c.lateral_tamanho = v,
                    "efeito_genio" => c.efeito_genio = sim,
                    "lateral_maximo" => c.lateral_maximo = v.parse().unwrap_or(5).clamp(1, 8),
                    "launchpad_adicionado" => c.launchpad_no_dock = sim,
                    "finder_adicionado" => c.finder_no_dock = sim,
                    "monitor_na_barra" => c.monitor_na_barra = sim,
                    "layouts" => c.layouts = sim,
                    "layout_monitor_principal" => {
                        if let Some(i) = crate::layouts::indice_layout(&v) {
                            c.layout_principal = i;
                        }
                    }
                    "layout_outros_monitores" => {
                        if let Some(i) = crate::layouts::indice_layout(&v) {
                            c.layout_outros = i;
                        }
                    }
                    "layout_espaco" => {
                        if let Some(i) = crate::layouts::indice_espaco(&v) {
                            c.layout_espaco = i;
                        }
                    }
                    "atalho_da_busca" if !v.is_empty() => c.atalho_busca = v,
                    "aparencia" if crate::tema::APARENCIAS.iter().any(|a| a.0 == v) => c.aparencia = v,
                    "cor_de_destaque" if crate::tema::DESTAQUES.iter().any(|a| a.0 == v) => c.destaque = v,
                    "transparencia" if crate::tema::TRANSPARENCIAS.iter().any(|a| a.0 == v) => c.transparencia = v,
                    "abrir_pastas_no_finder" => c.pastas_no_finder = sim,
                    "idioma" if crate::idioma::IDIOMAS.iter().any(|a| a.1 == v) => c.idioma = v,
                    k @ ("canto_superior_esquerdo" | "canto_superior_direito" | "canto_inferior_esquerdo" | "canto_inferior_direito")
                        if crate::cantos::ACOES.iter().any(|a| a.0 == v) =>
                    {
                        let i = ["canto_superior_esquerdo", "canto_superior_direito", "canto_inferior_esquerdo", "canto_inferior_direito"]
                            .iter()
                            .position(|n| *n == k)
                            .unwrap_or(0);
                        c.cantos[i] = v;
                    }
                    "atalho_da_visao_geral" if !v.is_empty() => c.atalho_visao = v,
                    "alternador_de_apps" => c.alternador = sim,
                    "posicao_do_dock" if matches!(v.as_str(), "baixo" | "esquerda" | "direita") => c.dock_posicao = v,
                    "previas_no_dock" => c.previas_no_dock = sim,
                    "lixeira_adicionada" => c.lixeira_no_dock = sim,
                    "reduzir_movimento" if matches!(v.as_str(), "automatico" | "sim" | "nao") => c.reduzir_movimento = v,
                    "menus_de_apps_electron" => c.menus_electron = sim,
                    "icones_uniformes" => c.icones_uniformes = sim,
                    "arredondar_janelas" => c.arredondar_janelas = sim,
                    _ => {}
                }
            }
        }
        c
    }

    pub fn salvar(&self) {
        let s = |b: bool| if b { "sim" } else { "nao" };
        let texto = format!(
            "# Configurações do Cais (sim/nao). Também dá para mudar em Preferências do Cais.\r\n\
             esconder_barra_de_tarefas={}\r\n\
             ocultar_automaticamente={}\r\n\
             ampliar_icones={}\r\n\
             # lateral (telinhas na esquerda), dock ou nao\r\n\
             minimizar_para={}\r\n\
             lateral_inclinada={}\r\n\
             lateral_reservar_espaco={}\r\n\
             lateral_maximo={}\r\n\
             # pequeno, medio ou grande\r\n\
             lateral_tamanho={}\r\n\
             efeito_genio={}\r\n\
             # alt+espaco, ctrl+espaco, ctrl+alt+espaco ou nenhum\r\n\
             atalho_da_busca={}\r\n\
             launchpad_adicionado={}\r\n\
             monitor_na_barra={}\r\n\
             finder_adicionado={}\r\n\
             # layouts de janelas: colunas2, colunas3, grade2x2, grade3x2, grade3x3, grande2, grande4\r\n\
             layouts={}\r\n\
             layout_monitor_principal={}\r\n\
             layout_outros_monitores={}\r\n\
             # nenhum, pequeno ou medio\r\n\
             layout_espaco={}\r\n\
             # automatico, escuro ou claro\r\n\
             aparencia={}\r\n\
             # azul, roxo, rosa, vermelho, laranja, amarelo, verde, grafite ou windows\r\n\
             cor_de_destaque={}\r\n\
             # pouca, media ou muita\r\n\
             transparencia={}\r\n\
             abrir_pastas_no_finder={}\r\n\
             # automatico (segue o Windows), pt ou en\r\n\
             idioma={}\r\n\
             # cantos ativos: nada, visao, mesa, apps, busca ou bloquear\r\n\
             canto_superior_esquerdo={}\r\n\
             canto_superior_direito={}\r\n\
             canto_inferior_esquerdo={}\r\n\
             canto_inferior_direito={}\r\n\
             # win+ctrl+seta_cima, ctrl+alt+seta_cima ou nenhum\r\n\
             atalho_da_visao_geral={}\r\n\
             alternador_de_apps={}\r\n\
             # baixo, esquerda ou direita\r\n\
             posicao_do_dock={}\r\n\
             previas_no_dock={}\r\n\
             lixeira_adicionada={}\r\n\
             # automatico (segue o Windows), sim ou nao\r\n\
             reduzir_movimento={}\r\n\
             menus_de_apps_electron={}\r\n\
             icones_uniformes={}\r\n\
             arredondar_janelas={}\r\n",
            s(self.esconder_barra),
            s(self.ocultar_auto),
            s(self.ampliar),
            self.minimizar_para,
            s(self.lateral_inclinar),
            s(self.lateral_reservar),
            self.lateral_maximo,
            self.lateral_tamanho,
            s(self.efeito_genio),
            self.atalho_busca,
            s(self.launchpad_no_dock),
            s(self.monitor_na_barra),
            s(self.finder_no_dock),
            s(self.layouts),
            crate::layouts::LAYOUTS[self.layout_principal].1,
            crate::layouts::LAYOUTS[self.layout_outros].1,
            crate::layouts::ESPACOS[self.layout_espaco].1,
            self.aparencia,
            self.destaque,
            self.transparencia,
            s(self.pastas_no_finder),
            self.idioma,
            self.cantos[0],
            self.cantos[1],
            self.cantos[2],
            self.cantos[3],
            self.atalho_visao,
            s(self.alternador),
            self.dock_posicao,
            s(self.previas_no_dock),
            s(self.lixeira_no_dock),
            self.reduzir_movimento,
            s(self.menus_electron),
            s(self.icones_uniformes),
            s(self.arredondar_janelas)
        );
        if let Some(dir) = self.arquivo.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.arquivo, texto);
    }

    pub fn pasta(&self) -> PathBuf {
        self.arquivo.parent().map(|p| p.to_path_buf()).unwrap_or_default()
    }

    /// Passa as escolhas de tema para o módulo de tema. Devolve se mudou.
    pub fn aplicar_tema(&self) -> bool {
        crate::tema::configurar(&self.aparencia, &self.destaque, &self.transparencia)
    }

    /// Passa as escolhas de layout para o módulo de layouts.
    pub fn aplicar_layouts(&self) {
        crate::layouts::configurar(self.layouts, self.layout_principal, self.layout_outros, self.layout_espaco);
    }

    pub fn opcoes_lateral(&self) -> crate::lateral::OpcoesLateral {
        crate::lateral::OpcoesLateral {
            inclinar: self.lateral_inclinar,
            reservar: self.lateral_reservar,
            maximo: self.lateral_maximo,
            escala: escala_lateral(&self.lateral_tamanho),
            genio: self.efeito_genio,
        }
    }
}

/// Pequeno = 75%, médio = 88%, grande = 100%.
pub fn escala_lateral(t: &str) -> f32 {
    match t {
        "pequeno" => 0.75,
        "medio" => 0.88,
        _ => 1.0,
    }
}

// ---------------------------------------------------------------------------
// Iniciar com o Windows: uma linha na chave "Run" do seu usuário
// (HKCU\Software\Microsoft\Windows\CurrentVersion\Run). Nada de serviço.

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::*;

const CHAVE_RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");

/// O cais.exe da pasta do programa (mesmo se este processo estiver rodando
/// de uma cópia renomeada, como "cais-em-uso.exe" durante uma atualização).
pub fn caminho_do_cais() -> PathBuf {
    let atual = std::env::current_exe().unwrap_or_default();
    let irmao = atual.with_file_name("cais.exe");
    if irmao.exists() {
        irmao
    } else {
        atual
    }
}

pub fn inicia_com_windows() -> bool {
    unsafe {
        let mut tam = 0u32;
        RegGetValueW(HKEY_CURRENT_USER, CHAVE_RUN, w!("Cais"), RRF_RT_REG_SZ, None, None, Some(&mut tam)).is_ok()
    }
}

pub fn definir_inicio_com_windows(sim: bool) {
    unsafe {
        let mut chave = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, CHAVE_RUN, 0, KEY_SET_VALUE, &mut chave).is_err() {
            return;
        }
        if sim {
            let exe = caminho_do_cais().to_string_lossy().into_owned();
            let valor: Vec<u16> = format!("\"{exe}\"").encode_utf16().chain(Some(0)).collect();
            let bytes = std::slice::from_raw_parts(valor.as_ptr() as *const u8, valor.len() * 2);
            let _ = RegSetValueExW(chave, w!("Cais"), 0, REG_SZ, Some(bytes));
        } else {
            let _ = RegDeleteValueW(chave, w!("Cais"));
        }
        let _ = RegCloseKey(chave);
    }
}
