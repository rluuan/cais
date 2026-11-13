//! Preferências do Cais: janela de vidro com barra lateral de seções
//! (Geral, Dock, Minimizar janelas, Busca e Launchpad, Sobre). Cada mudança
//! vale na hora e é salva no config.txt.

use crate::gpu::Gpu;
use crate::icones;
use crate::janela;
use crate::superficie::{arredondado, cor, retangulo, Superficie};
use crate::vidro::{Vidro, TINTA_PAINEL};
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const LARGURA: f32 = 780.0;
const ALTURA: f32 = 560.0;
const LATERAL: f32 = 210.0;
const ALT_LINHA: f32 = 62.0;
const HTCAPTION: isize = 2;
const HTCLIENT: isize = 1;

/// Valores que a tela mostra (vindos do config).
#[derive(Clone, Default)]
pub struct Valores {
    pub esconder_barra: bool,
    pub iniciar_com_windows: bool,
    pub ocultar_auto: bool,
    pub ampliar: bool,
    /// 0 = lateral, 1 = dock, 2 = não mexer.
    pub minimizar: usize,
    pub inclinar: bool,
    pub reservar: bool,
    pub maximo: usize,
    /// 0 = pequeno, 1 = médio, 2 = grande.
    pub tamanho_lateral: usize,
    pub genio: bool,
    /// Este Cais roda da pasta de programas (instalado).
    pub instalado_aqui: bool,
    /// Índice em ATALHOS.
    pub atalho: usize,
    pub atalho_ok: bool,
    pub monitor_na_barra: bool,
    pub layouts: bool,
    pub layout_principal: usize,
    pub layout_outros: usize,
    pub layout_espaco: usize,
    /// Índices em tema::APARENCIAS, DESTAQUES e TRANSPARENCIAS.
    pub aparencia: usize,
    pub destaque: usize,
    pub transparencia: usize,
    pub pastas_no_finder: bool,
    pub pasta: String,
    /// Índices em idioma::IDIOMAS, cantos::ACOES (um por canto),
    /// ATALHOS_VISAO, POSICOES e REDUZIR.
    pub idioma: usize,
    pub cantos: [usize; 4],
    pub atalho_visao: usize,
    pub alternador: bool,
    pub dock_posicao: usize,
    pub previas: bool,
    pub reduzir: usize,
    /// Há uma tarefa agendada que abre o Cais como administrador ao entrar.
    pub admin: bool,
    pub menus_electron: bool,
    pub icones_uniformes: bool,
    pub arredondar_janelas: bool,
}

/// (rótulo, valor no config). Os rótulos passam por `rotulo()` no inglês.
pub const ATALHOS: [(&str, &str); 4] = [
    ("Alt + Espaço", "alt+espaco"),
    ("Ctrl + Espaço", "ctrl+espaco"),
    ("Ctrl + Alt + Espaço", "ctrl+alt+espaco"),
    ("Nenhum (só a lupa da barra)", "nenhum"),
];
pub const ATALHOS_VISAO: [(&str, &str); 3] = [
    ("Win + Ctrl + ↑", "win+ctrl+seta_cima"),
    ("Ctrl + Alt + ↑", "ctrl+alt+seta_cima"),
    ("Nenhum (só os cantos ativos)", "nenhum"),
];
pub const POSICOES: [(&str, &str); 3] = [("Embaixo", "baixo"), ("Esquerda", "esquerda"), ("Direita", "direita")];
pub const REDUZIR: [(&str, &str); 3] = [("Automático (segue o Windows)", "automatico"), ("Sim", "sim"), ("Não", "nao")];
const MINIMIZAR: [(&str, &str); 3] = [
    ("Telinha no Palco (lateral)", "Thumbnail on the Stage (side)"),
    ("Miniatura no fim do dock", "Thumbnail at the end of the dock"),
    ("Não mexer (como o Windows)", "Leave it (like Windows)"),
];
const MAXIMOS: [&str; 6] = ["3", "4", "5", "6", "7", "8"];
pub const TAMANHOS: [(&str, &str); 3] = [("Pequeno", "pequeno"), ("Médio", "medio"), ("Grande", "grande")];

/// Rótulo de opção no idioma atual (inclusive os de atalho).
fn rot(pt: &'static str) -> String {
    if !crate::idioma::ingles() {
        return pt.to_string();
    }
    match pt {
        "Nenhum (só a lupa da barra)" => "None (only the bar's magnifier)".into(),
        "Nenhum (só os cantos ativos)" => "None (hot corners only)".into(),
        "Automático (segue o Windows)" => "Automatic (follows Windows)".into(),
        _ => crate::idioma::rotulo(pt).replace("Espaço", "Space"),
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Opcao {
    EsconderBarra,
    IniciarComWindows,
    OcultarAuto,
    Ampliar,
    Minimizar,
    Inclinar,
    Reservar,
    Maximo,
    TamanhoLateral,
    Genio,
    Atalho,
    AbrirPasta,
    PorLaunchpad,
    MonitorNaBarra,
    Reiniciar,
    Sair,
    Instalar,
    Desinstalar,
    Layouts,
    LayoutPrincipal,
    LayoutOutros,
    LayoutEspaco,
    Organizar,
    Aparencia,
    Destaque,
    Transparencia,
    PastasNoFinder,
    Idioma,
    AtalhoVisao,
    Alternador,
    /// Canto ativo 0..3.
    Canto(u8),
    DockPosicao,
    Previas,
    ReduzirMovimento,
    Admin,
    AbrirVisao,
    MenusElectron,
    IconesUniformes,
    ArredondarJanelas,
}

/// O que mudou (a App aplica e salva).
pub enum Mudanca {
    Alternar(Opcao),
    Escolher(Opcao, usize),
    Acao(Opcao),
}

#[derive(Clone, Copy, PartialEq)]
enum Secao {
    Geral,
    Aparencia,
    Dock,
    Minimizar,
    Visao,
    Layouts,
    Busca,
    Acessibilidade,
    Sobre,
}

/// (seção, nome em português, nome em inglês, cor do quadradinho)
const SECOES: [(Secao, &str, &str, (f32, f32, f32)); 9] = [
    (Secao::Geral, "Geral", "General", (0.55, 0.55, 0.6)),
    (Secao::Aparencia, "Aparência", "Appearance", (0.35, 0.35, 0.95)),
    (Secao::Dock, "Dock", "Dock", (0.2, 0.5, 1.0)),
    (Secao::Minimizar, "Palco e minimizar", "Stage & minimize", (0.95, 0.55, 0.15)),
    (Secao::Visao, "Visão Geral e cantos", "Overview & corners", (0.95, 0.3, 0.45)),
    (Secao::Layouts, "Layouts de janelas", "Window layouts", (0.1, 0.7, 0.75)),
    (Secao::Busca, "Busca e Apps", "Search & Apps", (0.6, 0.35, 0.95)),
    (Secao::Acessibilidade, "Acessibilidade", "Accessibility", (0.15, 0.55, 0.95)),
    (Secao::Sobre, "Sobre o Cais", "About Cais", (0.3, 0.75, 0.45)),
];

fn nome_secao(s: Secao) -> &'static str {
    SECOES.iter().find(|x| x.0 == s).map(|x| crate::idioma::t(x.1, x.2)).unwrap_or("")
}

enum Controle {
    Interruptor(bool),
    Escolha(String),
    Botao(&'static str),
    Nenhum,
}

struct Linha {
    titulo: String,
    descricao: String,
    controle: Controle,
    opcao: Option<Opcao>,
}

pub struct Preferencias {
    sup: Superficie,
    _vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    titulo: IDWriteTextFormat,
    texto: IDWriteTextFormat,
    pequeno: IDWriteTextFormat,
    grande: IDWriteTextFormat,
    secao: Secao,
    valores: Valores,
    hover: Option<usize>,
    hover_secao: Option<usize>,
    pub mudancas: Vec<Mudanca>,
    pub sujo: bool,
}

impl Drop for Preferencias {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.sup.hwnd);
        }
    }
}

impl Preferencias {
    pub fn abrir(gpu: &Gpu, valores: Valores) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_APPWINDOW)?;
        janela::preparar_vidro(hwnd);
        let e = janela::escala(hwnd);
        let (mon, trab) = janela::monitor_principal();
        let (l, a) = ((LARGURA * e).round() as i32, (ALTURA * e).round() as i32);
        let x = mon.left + (mon.right - mon.left - l) / 2;
        let y = trab.top + ((trab.bottom - trab.top - a) / 2).max(0);
        unsafe {
            let _ = SetWindowPos(hwnd, HWND_TOP, x, y, l, a, SWP_NOACTIVATE);
        }
        let mut sup = Superficie::new(gpu, hwnd)?;
        let _ = sup.arredondar(gpu, 12.0);
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, TINTA_PAINEL)?;
        vidro.definir(0.0, 0.0, l as f32, a as f32, 12.0 * e)?;
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let fonte = |peso, tam| -> Result<IDWriteTextFormat> {
                let f = gpu.dwrite.CreateTextFormat(
                    w!("Segoe UI Variable Text"),
                    None,
                    peso,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    tam,
                    crate::idioma::localidade(),
                )?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                Ok(f)
            };
            let titulo = fonte(DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.5)?;
            let texto = fonte(DWRITE_FONT_WEIGHT_NORMAL, 13.0)?;
            let pequeno = fonte(DWRITE_FONT_WEIGHT_NORMAL, 12.0)?;
            let grande = fonte(DWRITE_FONT_WEIGHT_SEMI_BOLD, 20.0)?;
            for f in [&titulo, &texto] {
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            }
            pequeno.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            let mut p = Self {
                sup,
                _vidro: vidro,
                pincel,
                titulo,
                texto,
                pequeno,
                grande,
                secao: Secao::Geral,
                valores,
                hover: None,
                hover_secao: None,
                mudancas: Vec::new(),
                sujo: true,
            };
            p.desenhar(gpu)?;
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
            p.sup.animar_entrada(gpu, 0.0, 0.97);
            Ok(p)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }

    pub fn trazer(&self) {
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.sup.hwnd);
        }
    }

    fn escala(&self) -> f32 {
        self.sup.dpi / 96.0
    }

    pub fn definir(&mut self, v: Valores) {
        self.valores = v;
        self.sujo = true;
    }

    fn linhas(&self) -> Vec<Linha> {
        let v = &self.valores;
        let l = |t: &str, d: &str, c: Controle, o: Option<Opcao>| Linha {
            titulo: t.into(),
            descricao: d.into(),
            controle: c,
            opcao: o,
        };
        use crate::idioma::t;
        use Controle::*;
        match self.secao {
            Secao::Geral => vec![
                l(t("Iniciar o Cais com o Windows", "Open Cais at login"), t("Abre sozinho quando você entra na sua conta.", "Starts by itself when you sign in."), Interruptor(v.iniciar_com_windows), Some(Opcao::IniciarComWindows)),
                l(t("Esconder a barra de tarefas do Windows", "Hide the Windows taskbar"), t("O dock e a barra do topo tomam o lugar dela. Ao sair do Cais ela volta.", "The dock and the menu bar take its place. It comes back when Cais quits."), Interruptor(v.esconder_barra), Some(Opcao::EsconderBarra)),
                l(t("Abrir pastas no Arquivos do Cais", "Open folders in Cais Files"), t("Pastas que abririam no Explorador abrem no Arquivos. Segure Shift para usar o Explorador.", "Folders that would open in File Explorer open in Files. Hold Shift to use File Explorer."), Interruptor(v.pastas_no_finder), Some(Opcao::PastasNoFinder)),
                l(t("CPU e memória na barra", "CPU and memory in the menu bar"), t("Mostra o uso do computador no topo (atualiza a cada 2 segundos).", "Shows how busy the computer is (updates every 2 seconds)."), Interruptor(v.monitor_na_barra), Some(Opcao::MonitorNaBarra)),
                l(t("Idioma", "Language"), t("Automático segue o idioma do Windows. Vale na hora.", "Automatic follows the Windows language. Applies right away."), Escolha(rot(crate::idioma::IDIOMAS[v.idioma.min(2)].0)), Some(Opcao::Idioma)),
                l(t("Reiniciar o Cais", "Restart Cais"), t("Fecha e abre de novo (pega a versão mais nova que estiver no disco).", "Quits and opens again (picks up the newest version on disk)."), Botao(t("Reiniciar", "Restart")), Some(Opcao::Reiniciar)),
                l(t("Sair do Cais", "Quit Cais"), t("Fecha o Cais e devolve a barra de tarefas do Windows.", "Closes Cais and gives the Windows taskbar back."), Botao(t("Sair", "Quit")), Some(Opcao::Sair)),
            ],
            Secao::Aparencia => {
                use crate::tema::{APARENCIAS, DESTAQUES, TRANSPARENCIAS};
                vec![
                    l(t("Aparência", "Appearance"), t("Automática segue o claro ou escuro do Windows.", "Automatic follows Windows light or dark mode."), Escolha(rot(APARENCIAS[v.aparencia.min(APARENCIAS.len() - 1)].1)), Some(Opcao::Aparencia)),
                    l(t("Cor de destaque", "Accent color"), t("Itens selecionados, interruptores e botões ligados.", "Selected items, switches and buttons."), Escolha(rot(DESTAQUES[v.destaque.min(DESTAQUES.len() - 1)].1)), Some(Opcao::Destaque)),
                    l(t("Ícones uniformes", "Uniform icons"), t("Todo app no mesmo formato arredondado, do mesmo tamanho, como no Mac.", "Every app in the same rounded shape and size, like on the Mac."), Interruptor(v.icones_uniformes), Some(Opcao::IconesUniformes)),
                    l(t("Arredondar todas as janelas", "Round all windows"), t("Cantos arredondados também nos apps que vêm retos. Ao desligar, as já abertas ficam assim até fechar.", "Rounded corners on apps that come square too. When turned off, open windows stay rounded until closed."), Interruptor(v.arredondar_janelas), Some(Opcao::ArredondarJanelas)),
                    l(t("Transparência do vidro", "Glass transparency"), t("Quanto do papel de parede aparece através do dock, da barra e dos painéis.", "How much of the wallpaper shows through the dock, the bar and the panels."), Escolha(rot(TRANSPARENCIAS[v.transparencia.min(TRANSPARENCIAS.len() - 1)].1)), Some(Opcao::Transparencia)),
                ]
            }
            Secao::Dock => vec![
                l(t("Posição na tela", "Position on screen"), t("Embaixo, ou de pé na esquerda ou na direita do monitor principal.", "Bottom, or standing on the left or right of the main display."), Escolha(rot(POSICOES[v.dock_posicao.min(2)].0)), Some(Opcao::DockPosicao)),
                l(t("Ocultar automaticamente", "Automatically hide"), t("O dock some e volta quando o mouse encosta na borda.", "The dock hides and comes back when the mouse touches the edge."), Interruptor(v.ocultar_auto), Some(Opcao::OcultarAuto)),
                l(t("Ampliar ícones sob o mouse", "Magnify icons under the mouse"), t("Os ícones crescem em onda quando o mouse passa por cima.", "Icons grow in a wave as the mouse passes over them."), Interruptor(v.ampliar), Some(Opcao::Ampliar)),
                l(t("Prévia das janelas", "Window previews"), t("Parar o mouse num app mostra as janelas abertas dele.", "Resting the mouse on an app shows its open windows."), Interruptor(v.previas), Some(Opcao::Previas)),
            ],
            Secao::Minimizar => vec![
                l(t("Ao minimizar uma janela", "When minimizing a window"), t("Para onde ela vai enquanto estiver minimizada.", "Where it goes while minimized."), Escolha(t(MINIMIZAR[v.minimizar.min(2)].0, MINIMIZAR[v.minimizar.min(2)].1).into()), Some(Opcao::Minimizar)),
                l(t("Tamanho das telinhas", "Thumbnail size"), t("Pequeno ou médio deixam a faixa do Palco mais estreita.", "Small or medium make the Stage strip narrower."), Escolha(rot(TAMANHOS[v.tamanho_lateral.min(2)].0)), Some(Opcao::TamanhoLateral)),
                l(t("Telinhas inclinadas (3D)", "Tilted thumbnails (3D)"), t("A janela aparece em perspectiva.", "The window is shown in perspective."), Interruptor(v.inclinar), Some(Opcao::Inclinar)),
                l(t("Efeito gênio", "Genie effect"), t("A janela escorre até a telinha ao minimizar e volta ao restaurar.", "The window flows into its thumbnail when minimized and back when restored."), Interruptor(v.genio), Some(Opcao::Genio)),
                l(t("Reservar a faixa do Palco", "Reserve the Stage strip"), t("Janelas maximizadas não ficam por cima das telinhas.", "Maximized windows don't cover the thumbnails."), Interruptor(v.reservar), Some(Opcao::Reservar)),
                l(t("Máximo de telinhas", "Maximum thumbnails"), t("As mais antigas saem da faixa (a janela continua minimizada).", "The oldest leave the strip (the window stays minimized)."), Escolha(v.maximo.to_string()), Some(Opcao::Maximo)),
            ],
            Secao::Visao => {
                use crate::cantos;
                let canto = |k: usize| {
                    let (pt, en) = cantos::NOMES[k];
                    l(t(pt, en), t("Encoste o mouse no canto da tela.", "Push the mouse into the corner of the screen."), Escolha(cantos::nome(v.cantos[k]).into()), Some(Opcao::Canto(k as u8)))
                };
                vec![
                    l(t("Atalho da Visão Geral", "Overview shortcut"), t("Todas as janelas lado a lado. Clique numa para trazê-la.", "All windows side by side. Click one to bring it forward."), Escolha(rot(ATALHOS_VISAO[v.atalho_visao.min(2)].0)), Some(Opcao::AtalhoVisao)),
                    l(t("Alternador de apps no Alt+Tab", "App switcher on Alt+Tab"), t("Alt+Tab mostra os ícones dos apps abertos, como o Cmd+Tab do Mac.", "Alt+Tab shows the icons of open apps, like Cmd+Tab on the Mac."), Interruptor(v.alternador), Some(Opcao::Alternador)),
                    canto(0),
                    canto(1),
                    canto(2),
                    canto(3),
                    l(t("Ver a Visão Geral agora", "See the Overview now"), t("Para experimentar sem atalho.", "To try it without a shortcut."), Botao(t("Abrir", "Open")), Some(Opcao::AbrirVisao)),
                ]
            }
            Secao::Layouts => {
                use crate::layouts::{ESPACOS, LAYOUTS};
                vec![
                    l(t("Organizar janelas em layouts", "Arrange windows in layouts"), t("Arraste uma janela segurando Shift: as áreas aparecem e ela se encaixa onde você soltar.", "Drag a window while holding Shift: the areas show up and it snaps where you drop it."), Interruptor(v.layouts), Some(Opcao::Layouts)),
                    l(t("Monitor principal", "Main display"), t("Como a tela é dividida.", "How the screen is divided."), Escolha(rot(LAYOUTS[v.layout_principal.min(LAYOUTS.len() - 1)].0)), Some(Opcao::LayoutPrincipal)),
                    l(t("Outros monitores", "Other displays"), t("Vale para todos os monitores além do principal.", "Applies to every display besides the main one."), Escolha(rot(LAYOUTS[v.layout_outros.min(LAYOUTS.len() - 1)].0)), Some(Opcao::LayoutOutros)),
                    l(t("Espaço entre as janelas", "Space between windows"), t("Uma folga em volta de cada área.", "A gap around each area."), Escolha(rot(ESPACOS[v.layout_espaco.min(ESPACOS.len() - 1)].0)), Some(Opcao::LayoutEspaco)),
                    l(t("Organizar agora", "Arrange now"), t("Distribui as janelas abertas de cada monitor pelas áreas (a da frente primeiro).", "Spreads the open windows of each display over the areas (frontmost first)."), Botao(t("Organizar", "Arrange")), Some(Opcao::Organizar)),
                ]
            }
            Secao::Busca => vec![
                l(
                    t("Atalho da busca", "Search shortcut"),
                    if v.atalho_ok {
                        t("Abre a busca de apps, ajustes, arquivos e contas de qualquer lugar.", "Opens the search for apps, settings, files and math from anywhere.")
                    } else {
                        t("Outro programa já usa esse atalho. A lupa da barra continua funcionando.", "Another program already uses this shortcut. The bar's magnifier still works.")
                    },
                    Escolha(rot(ATALHOS[v.atalho.min(3)].0)),
                    Some(Opcao::Atalho),
                ),
                l(t("Apps no dock", "Apps in the dock"), t("Se você tirou o Apps do dock, este botão coloca de volta.", "If you removed Apps from the dock, this button puts it back."), Botao(t("Pôr no dock", "Add to dock")), Some(Opcao::PorLaunchpad)),
            ],
            Secao::Acessibilidade => vec![
                l(t("Reduzir movimento", "Reduce motion"), t("Sem gênio, molas e zoom. Automático segue \"Efeitos de animação\" do Windows.", "No genie, springs or zoom. Automatic follows Windows \"Animation effects\"."), Escolha(rot(REDUZIR[v.reduzir.min(2)].0)), Some(Opcao::ReduzirMovimento)),
                l(t("Alto contraste", "High contrast"), t("Com o alto contraste do Windows ligado, o vidro fica opaco e o texto, mais forte.", "With Windows high contrast on, the glass turns solid and text gets stronger."), Nenhum, None),
                l(t("Teclado", "Keyboard"), t("Alt+Espaço busca, Win+Ctrl+↑ Visão Geral, Esc fecha qualquer painel do Cais.", "Alt+Space searches, Win+Ctrl+Up opens Overview, Esc closes any Cais panel."), Nenhum, None),
                l(
                    t("Menus de apps Electron na barra (experimental)", "Electron app menus in the bar (experimental)"),
                    t("Usa a Automação de Interface. O app pode ficar um pouco mais lento e o VS Code pode perguntar sobre leitor de tela.", "Uses UI Automation. The app may get a little slower and VS Code may ask about a screen reader."),
                    Interruptor(v.menus_electron),
                    Some(Opcao::MenusElectron),
                ),
                l(
                    t("Abrir como administrador ao entrar", "Open as administrator at login"),
                    t("Necessário para controlar apps que rodam como administrador. O Windows pede confirmação.", "Needed to work with apps running as administrator. Windows asks to confirm."),
                    Botao(if v.admin { t("Desligar", "Turn off") } else { t("Ligar", "Turn on") }),
                    Some(Opcao::Admin),
                ),
            ],
            Secao::Sobre => vec![
                l(&format!("Cais {}", env!("CARGO_PKG_VERSION")), t("Dock, barra de menu, Palco, Arquivos, Visão Geral e busca para Windows. Open source.", "Dock, menu bar, Stage, Files, Overview and search for Windows. Open source."), Nenhum, None),
                l(t("100% local", "100% local"), t("O Cais não tem nenhum código de rede: não envia nada, não baixa nada e não lê nada para ninguém.", "Cais has no network code at all: it sends nothing, downloads nothing and reads nothing for anyone."), Nenhum, None),
                l(t("Suas configurações", "Your settings"), &v.pasta, Botao(t("Abrir pasta", "Open folder")), Some(Opcao::AbrirPasta)),
                if v.instalado_aqui {
                    l(t("Desinstalar o Cais", "Uninstall Cais"), t("Remove o Cais deste computador. A barra de tarefas do Windows volta ao normal.", "Removes Cais from this computer. The Windows taskbar goes back to normal."), Botao(t("Desinstalar", "Uninstall")), Some(Opcao::Desinstalar))
                } else {
                    l(t("Instalar neste computador", "Install on this computer"), t("Copia o Cais para a pasta de programas, põe no menu Iniciar e em Aplicativos instalados.", "Copies Cais to the programs folder and adds it to Start and Installed apps."), Botao(t("Instalar", "Install")), Some(Opcao::Instalar))
                },
            ],
        }
    }

    /// Retângulo do controle da linha i (DIPs).
    fn ret_controle(&self, i: usize, c: &Controle) -> D2D_RECT_F {
        let y = 74.0 + i as f32 * ALT_LINHA + ALT_LINHA / 2.0;
        let dir = LARGURA - 28.0;
        match c {
            Controle::Interruptor(_) => retangulo(dir - 40.0, y - 12.0, dir, y + 12.0),
            Controle::Escolha(_) => retangulo(dir - 230.0, y - 14.0, dir, y + 14.0),
            Controle::Botao(_) => retangulo(dir - 110.0, y - 14.0, dir, y + 14.0),
            Controle::Nenhum => retangulo(0.0, 0.0, 0.0, 0.0),
        }
    }

    fn secao_em(&self, x: f32, y: f32) -> Option<usize> {
        if x > LATERAL - 10.0 || x < 10.0 {
            return None;
        }
        let i = ((y - 56.0) / 34.0).floor();
        (i >= 0.0 && (i as usize) < SECOES.len()).then_some(i as usize)
    }

    fn linha_em(&self, x: f32, y: f32) -> Option<usize> {
        self.linhas().iter().enumerate().find_map(|(i, l)| {
            let r = self.ret_controle(i, &l.controle);
            (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom).then_some(i)
        })
    }

    fn no_fechar(x: f32, y: f32) -> bool {
        (x - 22.0).powi(2) + (y - 22.0).powi(2) <= 64.0
    }

    pub fn mensagem(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let e = self.escala();
        let x = (lp.0 & 0xffff) as i16 as f32 / e;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / e;
        match msg {
            WM_NCHITTEST => {
                // Arrastar pela faixa de cima (menos o botão de fechar).
                let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                unsafe {
                    let _ = ScreenToClient(self.sup.hwnd, &mut pt);
                }
                let (cx, cy) = (pt.x as f32 / e, pt.y as f32 / e);
                let arrastar = cy < 46.0 && !Self::no_fechar(cx, cy);
                Some(LRESULT(if arrastar { HTCAPTION } else { HTCLIENT }))
            }
            WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
                unsafe {
                    let _ = PostMessageW(self.sup.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let (h, s) = (self.linha_em(x, y), self.secao_em(x, y));
                if h != self.hover || s != self.hover_secao {
                    self.hover = h;
                    self.hover_secao = s;
                    self.sujo = true;
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                if Self::no_fechar(x, y) {
                    unsafe {
                        let _ = PostMessageW(self.sup.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                } else if let Some(s) = self.secao_em(x, y) {
                    self.secao = SECOES[s].0;
                    self.hover = None;
                    self.sujo = true;
                } else if let Some(i) = self.linha_em(x, y) {
                    self.clicar(i);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn clicar(&mut self, i: usize) {
        let linhas = self.linhas();
        let Some(l) = linhas.get(i) else { return };
        let Some(op) = l.opcao else { return };
        match &l.controle {
            Controle::Interruptor(_) => self.mudancas.push(Mudanca::Alternar(op)),
            Controle::Botao(_) => self.mudancas.push(Mudanca::Acao(op)),
            Controle::Escolha(_) => {
                use crate::idioma::t;
                let v = &self.valores;
                let (lista, atual): (Vec<String>, usize) = match op {
                    Opcao::Minimizar => (MINIMIZAR.iter().map(|m| t(m.0, m.1).to_string()).collect(), v.minimizar),
                    Opcao::Maximo => (MAXIMOS.iter().map(|m| m.to_string()).collect(), v.maximo.saturating_sub(3)),
                    Opcao::TamanhoLateral => (TAMANHOS.iter().map(|x| rot(x.0)).collect(), v.tamanho_lateral),
                    Opcao::Atalho => (ATALHOS.iter().map(|a| rot(a.0)).collect(), v.atalho),
                    Opcao::AtalhoVisao => (ATALHOS_VISAO.iter().map(|a| rot(a.0)).collect(), v.atalho_visao),
                    Opcao::DockPosicao => (POSICOES.iter().map(|a| rot(a.0)).collect(), v.dock_posicao),
                    Opcao::ReduzirMovimento => (REDUZIR.iter().map(|a| rot(a.0)).collect(), v.reduzir),
                    Opcao::Idioma => (crate::idioma::IDIOMAS.iter().map(|a| rot(a.0)).collect(), v.idioma),
                    Opcao::Canto(k) => ((0..crate::cantos::ACOES.len()).map(|i| crate::cantos::nome(i).to_string()).collect(), v.cantos[k as usize]),
                    Opcao::LayoutPrincipal => (crate::layouts::LAYOUTS.iter().map(|l| rot(l.0)).collect(), v.layout_principal),
                    Opcao::LayoutOutros => (crate::layouts::LAYOUTS.iter().map(|l| rot(l.0)).collect(), v.layout_outros),
                    Opcao::LayoutEspaco => (crate::layouts::ESPACOS.iter().map(|e| rot(e.0)).collect(), v.layout_espaco),
                    Opcao::Aparencia => (crate::tema::APARENCIAS.iter().map(|a| rot(a.1)).collect(), v.aparencia),
                    Opcao::Destaque => (crate::tema::DESTAQUES.iter().map(|a| rot(a.1)).collect(), v.destaque),
                    Opcao::Transparencia => (crate::tema::TRANSPARENCIAS.iter().map(|a| rot(a.1)).collect(), v.transparencia),
                    _ => return,
                };
                let lista: Vec<&str> = lista.iter().map(|s| s.as_str()).collect();
                let r = self.ret_controle(i, &l.controle);
                if let Some(k) = self.lista(&lista, atual, r) {
                    let valor = if op == Opcao::Maximo { k + 3 } else { k };
                    self.mudancas.push(Mudanca::Escolher(op, valor));
                }
            }
            Controle::Nenhum => {}
        }
        self.sujo = true;
    }

    /// Lista de escolhas (menu do Windows no tema escuro) logo abaixo do botão.
    fn lista(&self, itens: &[&str], atual: usize, r: D2D_RECT_F) -> Option<usize> {
        let e = self.escala();
        unsafe {
            let m = CreatePopupMenu().ok()?;
            for (k, t) in itens.iter().enumerate() {
                let w: Vec<u16> = t.encode_utf16().chain(Some(0)).collect();
                let f = if k == atual { MF_STRING | MF_CHECKED } else { MF_STRING };
                let _ = AppendMenuW(m, f, k + 1, PCWSTR(w.as_ptr()));
            }
            let mut pt = POINT { x: (r.left * e) as i32, y: (r.bottom * e) as i32 + 2 };
            let _ = ClientToScreen(self.sup.hwnd, &mut pt);
            let c = TrackPopupMenu(m, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, 0, self.sup.hwnd, None).0;
            let _ = DestroyMenu(m);
            (c > 0).then(|| c as usize - 1)
        }
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        let linhas = self.linhas();
        let texto_layout = |s: &str, f: &IDWriteTextFormat, l: f32, a: f32| -> Result<IDWriteTextLayout> {
            let w: Vec<u16> = s.encode_utf16().collect();
            unsafe { gpu.dwrite.CreateTextLayout(&w, f, l, a) }
        };
        let cab = texto_layout(nome_secao(self.secao), &self.grande, 400.0, 40.0)?;
        let mut nomes = Vec::new();
        for (s, _, _, _) in SECOES.iter() {
            nomes.push(texto_layout(nome_secao(*s), &self.texto, LATERAL - 52.0, 34.0)?);
        }
        let larg_texto = LARGURA - LATERAL - 30.0 - 250.0;
        let mut corpo = Vec::new();
        for l in &linhas {
            let largura = if matches!(l.controle, Controle::Nenhum) { LARGURA - LATERAL - 56.0 } else { larg_texto };
            corpo.push((
                texto_layout(&l.titulo, &self.titulo, largura, 22.0)?,
                texto_layout(&l.descricao, &self.pequeno, largura, 34.0)?,
                match &l.controle {
                    Controle::Escolha(s) => Some(texto_layout(s, &self.texto, 190.0, 28.0)?),
                    Controle::Botao(s) => Some(texto_layout(s, &self.texto, 100.0, 28.0)?),
                    _ => None,
                },
            ));
        }
        let controles: Vec<D2D_RECT_F> = linhas.iter().enumerate().map(|(i, l)| self.ret_controle(i, &l.controle)).collect();
        let p = &self.pincel;
        let secao = self.secao;
        let (hover, hover_secao) = (self.hover, self.hover_secao);
        self.sup.desenhar(gpu, |dc| unsafe {
            // Barra lateral um pouco mais escura.
            p.SetColor(&cor(0.0, 0.0, 0.0, 0.18));
            dc.FillRectangle(&retangulo(0.0, 0.0, LATERAL, ALTURA), p);
            p.SetColor(&crate::tema::tinta(0.08));
            dc.FillRectangle(&retangulo(LATERAL, 0.0, LATERAL + 1.0, ALTURA), p);
            // Botão de fechar.
            p.SetColor(&cor(1.0, 0.37, 0.34, 1.0));
            dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: 22.0, y: 22.0 }, radiusX: 6.5, radiusY: 6.5 }, p);
            for (k, (s, _, _, rgb)) in SECOES.iter().enumerate() {
                let y = 56.0 + k as f32 * 34.0;
                let r = retangulo(10.0, y, LATERAL - 10.0, y + 30.0);
                if *s == secao {
                    p.SetColor(&crate::tema::destaque_a(0.85));
                    dc.FillRoundedRectangle(&arredondado(r, 7.0), p);
                } else if hover_secao == Some(k) {
                    p.SetColor(&crate::tema::tinta(0.08));
                    dc.FillRoundedRectangle(&arredondado(r, 7.0), p);
                }
                // Quadradinho colorido da seção.
                let q = retangulo(18.0, y + 6.0, 36.0, y + 24.0);
                p.SetColor(&cor(rgb.0, rgb.1, rgb.2, 1.0));
                dc.FillRoundedRectangle(&arredondado(q, 5.0), p);
                let branco = crate::tema::branco(0.95);
                let (cx, cy) = (27.0, y + 15.0);
                match s {
                    Secao::Geral => icones::engrenagem(dc, p, cx, cy, 0.6, branco),
                    Secao::Busca => icones::lupa(dc, p, cx, cy, 0.6, branco),
                    Secao::Dock => {
                        p.SetColor(&branco);
                        dc.FillRectangle(&retangulo(cx - 6.0, cy + 3.0, cx + 6.0, cy + 5.0), p);
                        for dx in [-4.0f32, 0.0, 4.0] {
                            dc.FillRectangle(&retangulo(cx + dx - 1.4, cy - 3.0, cx + dx + 1.4, cy + 1.6), p);
                        }
                    }
                    Secao::Minimizar => {
                        p.SetColor(&branco);
                        dc.DrawRectangle(&retangulo(cx - 5.5, cy - 4.0, cx + 2.5, cy + 2.0), p, 1.3, None);
                        dc.FillRectangle(&retangulo(cx - 1.0, cy, cx + 6.0, cy + 5.0), p);
                    }
                    Secao::Layouts => {
                        p.SetColor(&branco);
                        dc.DrawRectangle(&retangulo(cx - 6.0, cy - 5.0, cx - 0.8, cy + 5.0), p, 1.3, None);
                        dc.DrawRectangle(&retangulo(cx + 0.8, cy - 5.0, cx + 6.0, cy - 0.8), p, 1.3, None);
                        dc.DrawRectangle(&retangulo(cx + 0.8, cy + 0.8, cx + 6.0, cy + 5.0), p, 1.3, None);
                    }
                    Secao::Aparencia => {
                        // Círculo metade cheio (claro/escuro).
                        p.SetColor(&branco);
                        dc.DrawEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy }, radiusX: 5.5, radiusY: 5.5 }, p, 1.3, None);
                        dc.FillRectangle(&retangulo(cx, cy - 5.0, cx + 4.6, cy + 5.0), p);
                    }
                    Secao::Sobre => {
                        p.SetColor(&branco);
                        dc.FillRectangle(&retangulo(cx - 1.0, cy - 2.0, cx + 1.0, cy + 5.0), p);
                        dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy - 4.5 }, radiusX: 1.2, radiusY: 1.2 }, p);
                    }
                    Secao::Visao => {
                        // Quatro janelinhas lado a lado.
                        p.SetColor(&branco);
                        for (dx, dy) in [(-3.2f32, -2.6f32), (3.2, -2.6), (-3.2, 2.6), (3.2, 2.6)] {
                            dc.FillRoundedRectangle(&arredondado(retangulo(cx + dx - 2.6, cy + dy - 2.0, cx + dx + 2.6, cy + dy + 2.0), 0.8), p);
                        }
                    }
                    Secao::Acessibilidade => {
                        // Bonequinho de braços abertos.
                        p.SetColor(&branco);
                        dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: cy - 4.5 }, radiusX: 1.6, radiusY: 1.6 }, p);
                        dc.FillRectangle(&retangulo(cx - 5.5, cy - 2.0, cx + 5.5, cy - 0.6), p);
                        dc.FillRectangle(&retangulo(cx - 1.0, cy - 2.0, cx + 1.0, cy + 2.2), p);
                        dc.DrawLine(D2D_POINT_2F { x: cx - 0.6, y: cy + 1.8 }, D2D_POINT_2F { x: cx - 3.0, y: cy + 5.5 }, p, 1.5, None);
                        dc.DrawLine(D2D_POINT_2F { x: cx + 0.6, y: cy + 1.8 }, D2D_POINT_2F { x: cx + 3.0, y: cy + 5.5 }, p, 1.5, None);
                    }
                }
                p.SetColor(&crate::tema::tinta_sobre(*s == secao, 0.95));
                dc.DrawTextLayout(D2D_POINT_2F { x: 46.0, y: y - 2.0 }, &nomes[k], p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            }

            let x0 = LATERAL + 28.0;
            p.SetColor(&crate::tema::tinta(0.96));
            dc.DrawTextLayout(D2D_POINT_2F { x: x0, y: 14.0 }, &cab, p, D2D1_DRAW_TEXT_OPTIONS_NONE);
            for (i, l) in linhas.iter().enumerate() {
                let y = 74.0 + i as f32 * ALT_LINHA;
                if i > 0 {
                    p.SetColor(&crate::tema::tinta(0.07));
                    dc.FillRectangle(&retangulo(x0, y - 0.5, LARGURA - 28.0, y + 0.5), p);
                }
                let (lt, ld, lc) = &corpo[i];
                p.SetColor(&crate::tema::tinta(0.95));
                dc.DrawTextLayout(D2D_POINT_2F { x: x0, y: y + 8.0 }, lt, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                p.SetColor(&crate::tema::tinta(0.55));
                dc.DrawTextLayout(D2D_POINT_2F { x: x0, y: y + 31.0 }, ld, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                let r = controles[i];
                let realce = if hover == Some(i) { 0.06 } else { 0.0 };
                match &l.controle {
                    Controle::Interruptor(lig) => {
                        p.SetColor(&if *lig { crate::tema::destaque_a(1.0) } else { crate::tema::tinta(0.2 + realce) });
                        dc.FillRoundedRectangle(&arredondado(r, 12.0), p);
                        let bx = if *lig { r.right - 12.0 } else { r.left + 12.0 };
                        p.SetColor(&crate::tema::branco(1.0));
                        dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: bx, y: (r.top + r.bottom) / 2.0 }, radiusX: 10.0, radiusY: 10.0 }, p);
                    }
                    Controle::Escolha(_) | Controle::Botao(_) => {
                        p.SetColor(&crate::tema::tinta(0.12 + realce));
                        dc.FillRoundedRectangle(&arredondado(r, 7.0), p);
                        p.SetColor(&crate::tema::tinta(0.14));
                        dc.DrawRoundedRectangle(&arredondado(r, 7.0), p, 1.0, None);
                        if let Some(lc) = lc {
                            p.SetColor(&crate::tema::tinta(0.95));
                            dc.DrawTextLayout(D2D_POINT_2F { x: r.left + 12.0, y: r.top }, lc, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                        }
                        if matches!(l.controle, Controle::Escolha(_)) {
                            p.SetColor(&crate::tema::tinta(0.6));
                            icones::seta(dc, p, r.right - 14.0, (r.top + r.bottom) / 2.0, true);
                        }
                    }
                    Controle::Nenhum => {}
                }
            }
        })?;
        self.sujo = false;
        Ok(())
    }
}
