#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Cais: dock e barra de menu para Windows, leve e 100% local.
//! Este programa não contém nenhum código de rede.
//!
//! No dock:
//!   clique              abre o app, traz para frente ou minimiza
//!   clique direito      menu (Manter no Cais, Remover, Fechar, Sair do Cais)
//!   clique do meio      mostra/esconde o medidor de consumo
//!
//! Linha de comando:
//!   --medir <seg>     roda por <seg> segundos e imprime um relatório de consumo
//!   --estresse        força a animação o tempo todo (pior caso)
//!   --medidor         já abre com o medidor visível
//!   --saida <arquivo> grava o relatório também em arquivo

mod apps;
mod arquivos;
mod arredondar;
mod bandeja;
mod barra;
mod busca;
mod campo;
mod catalogo;
mod central;
mod config;
mod dock;
mod calendario;
mod dica;
mod pastas;
mod tema;
mod encaixe;
mod finder;
mod genio;
mod gpu;
mod instalador;
mod menus_uia;
mod icones;
mod janela;
mod lateral;
mod layouts;
mod preferencias;
mod launchpad;
mod medidor;
mod mola;
mod monitor;
mod sistema;
mod superficie;
mod teste3d;
mod topo;
mod vidro;
mod alternador;
mod areas;
mod cantos;
mod entrada;
mod idioma;
mod indice;
mod placa;
mod previa;
mod visao;

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;
use windows::core::Result;
use windows::Win32::Foundation::*;
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const TIMER_MEDIDOR: usize = 1;
const TIMER_MEDICAO: usize = 2;
const TIMER_JANELAS: usize = 3;
const MA_NOACTIVATE: isize = 3;
const WM_MOUSELEAVE: u32 = 0x02A3;
/// A barra de tarefas tentou reaparecer: esconder de novo.
const WM_APP_BARRA: u32 = WM_APP + 2;
const ABN_POSCHANGED: usize = 1;
const ABN_FULLSCREENAPP: usize = 2;

// Eventos de janela do Windows que interessam ao dock.
const EV_DESTRUIDA: u32 = 0x8001;
const EV_MOSTRADA: u32 = 0x8002;
const EV_ESCONDIDA: u32 = 0x8003;
const EV_NOME: u32 = 0x800C;
const EV_OCULTADA: u32 = 0x8017;
const EV_REVELADA: u32 = 0x8018;
const WINEVENT_FORA_DO_PROCESSO: u32 = 0x0000;
const WINEVENT_IGNORAR_PROPRIO: u32 = 0x0002;

static DOCK_HWND: AtomicIsize = AtomicIsize::new(0);
static TOPO_HWND: AtomicIsize = AtomicIsize::new(0);
const EV_PRIMEIRO_PLANO: u32 = 0x0003;
const EV_MINIMIZOU: u32 = 0x0016;
const EV_RESTAUROU: u32 = 0x0017;
/// Uma janela minimizou (lParam 1) ou voltou (lParam 0); wParam = HWND.
const WM_APP_MINIMIZACAO: u32 = WM_APP + 5;
/// Começou (lParam 1) ou terminou (lParam 0) o arrasto de uma janela.
const WM_APP_ARRASTO: u32 = WM_APP + 16;
/// Uma janela nova do Explorador teve a pasta lida (pastas.rs).
const WM_APP_PASTA: u32 = WM_APP + 18;
const EV_ARRASTO_INICIO: u32 = 0x000A;
const EV_ARRASTO_FIM: u32 = 0x000B;
/// O Launchpad pediu para fechar (destruído fora da própria mensagem).
const WM_APP_FECHAR_LAUNCHPAD: u32 = WM_APP + 7;
/// Uma janela do Finder pediu para fechar.
const WM_APP_FECHAR_FINDER: u32 = WM_APP + 12;
const ATALHO_BUSCA: i32 = 1;
const ATALHO_VISAO: i32 = 2;
/// Ctrl+F3: foco do teclado no dock (como no Mac).
const ATALHO_DOCK: i32 = 3;
/// Teclas do modo teclado do dock (só registradas enquanto ele está ativo).
const TECLAS_DOCK: [(i32, u32); 6] = [
    (10, 0x25), // seta esquerda
    (11, 0x27), // seta direita
    (12, 0x26), // seta cima
    (13, 0x28), // seta baixo
    (14, 0x0D), // Enter
    (15, 0x1B), // Esc
];
/// Canto ativo: espera um instante para confirmar que o mouse ficou lá.
const TIMER_CANTO: usize = 32;
/// Prévia das janelas de um app do dock (mouse parado no ícone).
const TIMER_PREVIA: usize = 33;
/// Fecha a prévia quando o mouse sai dela e do dock.
const TIMER_PREVIA_SAIR: usize = 34;

/// Teste (cais.exe --testar-bandeja): liga só a bandeja por 8 s, lista os
/// ícones que chegaram e confere se o Explorer continua respondendo.
fn testar_bandeja() -> Result<()> {
    use windows::Win32::UI::Shell::*;
    janela::registrar(Some(wndproc))?;
    let aviso = janela::criar(WS_EX_TOOLWINDOW)?;
    if !bandeja::iniciar(aviso) {
        println!("bandeja: não criou a janela");
        return Ok(());
    }
    let fim = Instant::now() + std::time::Duration::from_secs(8);
    let mut msg = MSG::default();
    while Instant::now() < fim {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let primeira = unsafe { FindWindowW(windows::core::w!("Shell_TrayWnd"), windows::core::PCWSTR::null()) }.unwrap_or_default();
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(primeira, Some(&mut pid)) };
    println!("primeira Shell_TrayWnd é do Cais: {}", pid == std::process::id());
    // O Explorer ainda responde a uma barra de aplicativo (passando por nós)?
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    let r = unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut d) };
    println!("ABM_GETTASKBARPOS: {} ({},{},{},{})", r, d.rc.left, d.rc.top, d.rc.right, d.rc.bottom);
    let v = bandeja::icones();
    println!("icones: {}", v.len());
    for i in &v {
        println!("  v{} {}", i.versao, i.dica.lines().next().unwrap_or(""));
    }
    bandeja::encerrar();
    Ok(())
}

/// Pedido de "Reiniciar o Cais": sai limpo e abre de novo no fim do main.
static REINICIAR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn reiniciar() {
    REINICIAR.store(true, Ordering::Relaxed);
    unsafe { PostQuitMessage(0) };
}

/// Depois de instalar: reabrir a partir da pasta de programas.
static REABRIR_EM: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Mensagem "Cais.Sair" (o desinstalador pede para o Cais aberto sair).
static MSG_SAIR: AtomicIsize = AtomicIsize::new(-1);

/// Desinstalação pedida nas Preferências: termina depois de sair
/// (Some(apagar as configurações?)).
static DESINSTALAR_AO_SAIR: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

fn modo_de(nome: &str) -> u8 {
    match nome {
        "dock" => 1,
        "nao" => 2,
        _ => 0,
    }
}

/// Registra o atalho global da busca ("alt+espaco", "ctrl+espaco"...).
/// Se outro programa já usa a combinação, a lupa da barra continua valendo.
fn registrar_atalho(hwnd: HWND, texto: &str) -> bool {
    registrar_atalho_id(hwnd, ATALHO_BUSCA, texto)
}

fn registrar_atalho_id(hwnd: HWND, id: i32, texto: &str) -> bool {
    unsafe {
        let _ = UnregisterHotKey(hwnd, id);
    }
    let mut mods = MOD_NOREPEAT;
    let mut tecla = None;
    for parte in texto.split('+').map(|p| p.trim().to_lowercase()) {
        match parte.as_str() {
            "alt" => mods |= MOD_ALT,
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "shift" => mods |= MOD_SHIFT,
            "win" => mods |= MOD_WIN,
            "espaco" | "espaço" | "space" => tecla = Some(VK_SPACE.0 as u32),
            "seta_cima" | "up" => tecla = Some(VK_UP.0 as u32),
            "seta_baixo" | "down" => tecla = Some(VK_DOWN.0 as u32),
            "tab" => tecla = Some(VK_TAB.0 as u32),
            p if p.len() == 1 && p.chars().all(|c| c.is_ascii_alphanumeric()) => {
                tecla = Some(p.to_ascii_uppercase().chars().next().unwrap() as u32)
            }
            p if p.starts_with('f') && p[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) => {
                tecla = Some(VK_F1.0 as u32 + p[1..].parse::<u32>().unwrap() - 1)
            }
            _ => {}
        }
    }
    match tecla {
        Some(vk) => unsafe { RegisterHotKey(hwnd, id, mods, vk).is_ok() },
        None => false,
    }
}
/// Mensagem que o Windows manda quando o Explorer (re)cria a barra.
static TASKBAR_CREATED: AtomicIsize = AtomicIsize::new(-1);
static RELEITURAS: AtomicIsize = AtomicIsize::new(0);
static EVENTOS: [AtomicIsize; 4] = [AtomicIsize::new(0), AtomicIsize::new(0), AtomicIsize::new(0), AtomicIsize::new(0)];

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static CONHECIDAS: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
}

#[derive(Default)]
struct Opcoes {
    medir_seg: Option<u32>,
    estresse: bool,
    medidor: bool,
    saida: Option<String>,
    restaurar_barra: bool,
    testar_3d: bool,
    testar_genio: bool,
    testar_bandeja: bool,
    testar_finder: bool,
    testar_lateral: bool,
    instalar: bool,
    testar_menus: bool,
    testar_encaixe: bool,
    desinstalar: bool,
}

impl Opcoes {
    /// No modo de medição o Cais não mexe na barra nem nas suas janelas.
    fn medindo(&self) -> bool {
        self.medir_seg.is_some()
    }
}

fn ler_opcoes() -> Opcoes {
    let mut o = Opcoes::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--medir" => o.medir_seg = Some(args.next().and_then(|s| s.parse().ok()).unwrap_or(10)),
            "--estresse" => o.estresse = true,
            "--medidor" => o.medidor = true,
            "--saida" => o.saida = args.next(),
            "--restaurar-barra" | "--restore-taskbar" => o.restaurar_barra = true,
            "--install" => o.instalar = true,
            "--uninstall" => o.desinstalar = true,
            "--testar-3d" => o.testar_3d = true,
            "--testar-genio" => o.testar_genio = true,
            "--testar-bandeja" => o.testar_bandeja = true,
            "--testar-finder" => o.testar_finder = true,
            "--testar-lateral" => o.testar_lateral = true,
            "--instalar" => o.instalar = true,
            "--testar-menus" => o.testar_menus = true,
            "--testar-encaixe" => o.testar_encaixe = true,
            "--desinstalar" => o.desinstalar = true,
            _ => {}
        }
    }
    o
}

struct App {
    gpu: gpu::Gpu,
    dock: dock::Dock,
    topo: topo::Topo,
    medidor: medidor::Medidor,
    sistema: sistema::Sistema,
    /// Criada no primeiro clique (quem nunca abre a Central não paga por ela).
    central: Option<central::Central>,
    catalogo: catalogo::Catalogo,
    /// Busca: criada no primeiro uso. Launchpad: só existe enquanto aberto.
    busca: Option<busca::Busca>,
    launchpad: Option<launchpad::Launchpad>,
    /// Janelas do Finder abertas.
    finders: Vec<finder::Finder>,
    /// Telinhas das janelas minimizadas na esquerda (estilo Stage Manager).
    laterais: Vec<lateral::Lateral>,
    /// Efeitos gênio rodando agora (cada um some sozinho no fim).
    genios: Vec<genio::Genio>,
    /// Janela do gênio já pronta (escondida) para o próximo minimizar.
    palco: Option<genio::Palco>,
    /// Arrastando uma janela (layouts: Shift mostra as áreas).
    arrasto: Option<layouts::Arrasto>,
    /// Barras do topo dos monitores secundários (a do principal é `topo`).
    topos_extra: Vec<topo::Topo>,
    /// Para saber se os monitores mudaram (ids dos HMONITOR).
    monitores_conhecidos: Vec<isize>,
    /// Barra que abriu a Central de Controle (0 = principal).
    central_dono: usize,
    /// Janela de Preferências (só existe aberta).
    preferencias: Option<preferencias::Preferencias>,
    /// Ícones da bandeja (cópia da lista) e o bitmap de cada HICON.
    bandeja_icones: Vec<bandeja::Icone>,
    /// Monitor do sistema: leitor, painel (criado no primeiro uso) e a
    /// barra que abriu o painel.
    leitor: monitor::Leitor,
    painel_monitor: Option<monitor::Painel>,
    monitor_dono: usize,
    /// Calendário do relógio (criado no primeiro clique).
    calendario: Option<calendario::Calendario>,
    bandeja_bitmaps: std::collections::HashMap<isize, Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>>,
    ultimo_clique_bandeja: Option<(usize, Instant)>,
    /// O atalho da busca foi aceito pelo Windows.
    atalho_ok: bool,
    opcoes: Opcoes,
    config: config::Config,
    inicio: Instant,
    ultimo_quadro: Option<Instant>,
    primeiro_quadro_ms: f64,
    base_medicao: Option<medidor::Amostra>,
    rastreando_mouse: bool,
    /// Visão Geral (só existe aberta) e alternador de apps (só com Alt+Tab).
    visao: Option<visao::Visao>,
    alternador: Option<alternador::Alternador>,
    /// Canto ativo esperando confirmação.
    canto_pendente: Option<usize>,
    /// Prévia das janelas de um app do dock.
    previa: Option<previa::Previa>,
    /// Pontos das áreas de trabalho virtuais na barra.
    areas: areas::Areas,
}

impl App {
    fn new(opcoes: Opcoes, inicio: Instant) -> Result<Self> {
        let config = config::Config::carregar();
        // Idioma e tema antes de criar qualquer janela (todas nascem certas).
        idioma::configurar(&config.idioma);
        placa::ligar(config.icones_uniformes);
        arredondar::ligar(config.arredondar_janelas);
        config.aplicar_tema();
        janela::menus_no_tema();
        let medindo = opcoes.medindo();
        if !medindo {
            config.salvar();
            if config.esconder_barra {
                barra::esconder();
            } else {
                barra::mostrar(); // caso uma execução anterior tenha travado
            }
        }
        let gpu = gpu::Gpu::new()?;
        let dock = dock::Dock::new(
            &gpu,
            config.ocultar_auto && !medindo,
            config.esconder_barra && !medindo,
            !medindo,
            dock::Posicao::de(&config.dock_posicao),
        )?;
        let topo = topo::Topo::new(&gpu, !medindo, janela::hmonitor_principal())?;
        let mut dock = dock;
        dock.minimizar_no_dock = config.minimizar_para == "dock";
        dock.modo_minimizar = modo_de(&config.minimizar_para);
        dock.ler_minimizadas();
        dock.ampliacao = if config.ampliar { dock::AMPLIACAO } else { 1.0 };
        let lateral = lateral::Lateral::new(&gpu, !medindo, config.opcoes_lateral(), janela::hmonitor_principal())?;
        if !medindo && !config.launchpad_no_dock {
            dock.modelo.adicionar_launchpad();
        }
        if !medindo && !config.finder_no_dock {
            dock.modelo.adicionar_finder();
        }
        if !medindo && !config.lixeira_no_dock {
            dock.modelo.adicionar_pilhas();
        }
        let catalogo = catalogo::Catalogo::new(topo.hwnd(), dock.escala());
        let medidor = medidor::Medidor::new(&gpu)?;
        let sistema = sistema::Sistema::iniciar(topo.hwnd());
        let mut app = App {
            gpu,
            dock,
            topo,
            medidor,
            sistema,
            central: None,
            catalogo,
            busca: None,
            launchpad: None,
            finders: Vec::new(),
            laterais: vec![lateral],
            genios: Vec::new(),
            palco: None,
            arrasto: None,
            topos_extra: Vec::new(),
            monitores_conhecidos: Vec::new(),
            central_dono: 0,
            preferencias: None,
            bandeja_icones: Vec::new(),
            leitor: monitor::Leitor::default(),
            painel_monitor: None,
            monitor_dono: 0,
            calendario: None,
            bandeja_bitmaps: std::collections::HashMap::new(),
            ultimo_clique_bandeja: None,
            atalho_ok: false,
            opcoes,
            config,
            inicio,
            ultimo_quadro: None,
            primeiro_quadro_ms: 0.0,
            base_medicao: None,
            rastreando_mouse: false,
            visao: None,
            alternador: None,
            canto_pendente: None,
            previa: None,
            areas: areas::Areas::default(),
        };
        app.dock.desenhar(&app.gpu)?;
        app.topo.desenhar(&app.gpu)?;
        app.dock.mostrar();
        app.topo.mostrar();
        TOPO_HWND.store(app.topo.hwnd().0 as isize, Ordering::Relaxed);
        menus_uia::definir_aviso(app.topo.hwnd());
        app.primeiro_quadro_ms = inicio.elapsed().as_secs_f64() * 1000.0;
        DOCK_HWND.store(app.dock.hwnd().0 as isize, Ordering::Relaxed);
        unsafe {
            // O desinstalador pode rodar sem administrador e este Cais com.
            let _ = windows::Win32::UI::WindowsAndMessaging::ChangeWindowMessageFilterEx(
                app.dock.hwnd(),
                MSG_SAIR.load(Ordering::Relaxed) as u32,
                MSGFLT_ALLOW,
                None,
            );
        }
        if !app.opcoes.medindo() {
            // Barra do topo e faixa das telinhas nos outros monitores.
            app.atualizar_monitores();
        }
        app.lembrar_janelas();
        medidor::enxugar_memoria();
        app.sistema.remetente().send(sistema::Cmd::Comecar).ok();
        if !app.opcoes.medindo() {
            // Ícones da bandeja na barra do topo (os programas reenviam).
            bandeja::iniciar(app.topo.hwnd());
            app.agendar_monitor();
            if !app.config.launchpad_no_dock || !app.config.finder_no_dock || !app.config.lixeira_no_dock {
                app.config.launchpad_no_dock = true;
                app.config.finder_no_dock = true;
                app.config.lixeira_no_dock = true;
                app.config.salvar();
            }
            app.atalho_ok = registrar_atalho(app.topo.hwnd(), &app.config.atalho_busca);
            registrar_atalho_id(app.topo.hwnd(), ATALHO_VISAO, &app.config.atalho_visao);
            registrar_atalho_id(app.topo.hwnd(), ATALHO_DOCK, "ctrl+f3");
            app.ligar_entrada();
            app.areas.ligar(app.topo.hwnd());
            instalador::conferir_admin();
            menus_uia::ler_electron(app.config.menus_electron);
            animacao_do_windows(app.usar_genio());
            if app.usar_genio() {
                app.palco = genio::Palco::new(&app.gpu);
            }
            app.config.aplicar_layouts();
            // Janelas maximizadas que ficaram por baixo da barra ou do dock.
            encaixe::definir_aviso(app.dock.hwnd());
            pastas::definir_aviso(app.dock.hwnd(), WM_APP_PASTA);
            pastas::ligar(app.config.pastas_no_finder);
        }
        if app.opcoes.medidor {
            app.medidor.alternar(false);
        }
        if let Some(seg) = app.opcoes.medir_seg {
            app.base_medicao = Some(medidor::amostrar());
            unsafe {
                SetTimer(app.dock.hwnd(), TIMER_MEDICAO, seg * 1000, None);
            }
        }
        Ok(app)
    }

    fn alternar_barra(&mut self) {
        self.config.esconder_barra = !self.config.esconder_barra;
        self.config.salvar();
        if self.config.esconder_barra {
            barra::esconder();
        } else {
            barra::mostrar();
        }
        self.dock.barra_escondida = self.config.esconder_barra;
        let _ = self.dock.reposicionar(&self.gpu);
    }

    fn alternar_auto(&mut self) {
        self.config.ocultar_auto = !self.config.ocultar_auto;
        self.config.salvar();
        self.dock.definir_auto(&self.gpu, self.config.ocultar_auto);
    }

    /// Ao sair (ou ao desligar o PC): devolve a barra e a reserva de espaço.
    fn encerrar(&self) {
        bandeja::encerrar();
        self.sistema.encerrar();
        self.dock.sair_do_windows();
        self.topo.sair_do_windows();
        for t in &self.topos_extra {
            t.sair_do_windows();
        }
        for l in &self.laterais {
            l.sair_do_windows();
        }
        animacao_do_windows(false);
        genio::mostrar_todas();
        if !self.opcoes.medindo() {
            barra::mostrar();
        }
    }

    /// O gênio vale com as janelas indo para a faixa lateral.
    fn usar_genio(&self) -> bool {
        self.config.efeito_genio && self.config.minimizar_para == "lateral" && !self.opcoes.medindo() && !self.movimento_reduzido()
    }

    /// "Reduzir movimento": pela escolha do Cais ou, no automático, pela
    /// opção "Efeitos de animação" do Windows.
    fn movimento_reduzido(&self) -> bool {
        match self.config.reduzir_movimento.as_str() {
            "sim" => true,
            "nao" => false,
            _ => !janela::animacoes_do_windows(),
        }
    }

    /// Liga ou desliga os ganchos de teclado (alternador) e mouse (cantos).
    fn ligar_entrada(&self) {
        entrada::configurar(self.dock.hwnd(), self.config.alternador, cantos::algum_ligado(&self.config.cantos));
    }

    // ----- Visão Geral, alternador e cantos ativos -----

    fn alternar_visao(&mut self) {
        if let Some(v) = &mut self.visao {
            v.fechar(None);
            return;
        }
        self.fechar_central();
        self.fechar_launchpad();
        self.fechar_previa();
        if let Some(b) = &mut self.busca {
            b.fechar();
        }
        if let Ok(Some(v)) = visao::Visao::abrir(&self.gpu, self.movimento_reduzido()) {
            self.visao = Some(v);
        }
    }

    fn evento_alternador(&mut self, ev: usize) {
        match ev {
            entrada::EV_ABRIR => {
                let apps = self.apps_para_alternar();
                if apps.len() < 1 {
                    return;
                }
                self.alternador = alternador::Alternador::abrir(&self.gpu, apps).ok();
                unsafe {
                    SetTimer(self.dock.hwnd(), alternador::TIMER_ALTERNADOR, 130, None);
                }
            }
            entrada::EV_PROXIMO => {
                if let Some(a) = &mut self.alternador {
                    a.avancar(1);
                }
            }
            entrada::EV_ANTERIOR => {
                if let Some(a) = &mut self.alternador {
                    a.avancar(-1);
                }
            }
            entrada::EV_CONFIRMAR => {
                if let Some(h) = self.alternador.take().and_then(|a| a.janela_escolhida()) {
                    apps::trazer_janela(h);
                }
            }
            _ => self.alternador = None,
        }
    }

    /// Apps com janela aberta, do usado por último ao mais antigo.
    fn apps_para_alternar(&self) -> Vec<alternador::App> {
        let ordem: Vec<HWND> = apps::enumerar_janelas().into_iter().map(|(h, _)| h).collect();
        let mut v: Vec<(usize, alternador::App)> = self
            .dock
            .modelo
            .itens
            .iter()
            .filter(|i| !i.janelas.is_empty())
            .filter_map(|i| {
                let (pos, h) = i
                    .janelas
                    .iter()
                    .filter_map(|h| ordem.iter().position(|o| o == h).map(|p| (p, *h)))
                    .min_by_key(|(p, _)| *p)?;
                Some((pos, alternador::App { nome: i.nome.clone(), icone: i.icone.clone(), janela: h }))
            })
            .collect();
        v.sort_by_key(|(p, _)| *p);
        v.into_iter().map(|(_, a)| a).collect()
    }

    /// O mouse parou num canto: confirma e faz a ação daquele canto.
    fn acao_do_canto(&mut self, c: usize) {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        let m = unsafe { windows::Win32::Graphics::Gdi::MonitorFromPoint(p, windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST) };
        let (mon, _) = janela::info_monitor(m);
        if entrada::canto_de(p, mon, 3) != Some(c) {
            return; // só passou de raspão
        }
        match self.config.cantos[c].as_str() {
            "visao" => self.alternar_visao(),
            "mesa" => topo::teclas_windows(windows::Win32::UI::Input::KeyboardAndMouse::VK_D),
            "apps" => self.alternar_launchpad(),
            "busca" => self.alternar_busca(),
            "bloquear" => unsafe {
                let _ = windows::Win32::System::Shutdown::LockWorkStation();
            },
            _ => {}
        }
    }

    // ----- prévia das janelas no dock -----

    fn abrir_previa(&mut self) {
        let Some((janelas, ancora)) = self.dock.pedido_de_previa() else { return };
        if let Some(p) = &self.previa {
            if p.janelas() == janelas.as_slice() {
                return;
            }
        }
        self.previa = previa::Previa::abrir(&self.gpu, &janelas, ancora, self.dock.posicao).ok();
        if self.previa.is_some() {
            unsafe {
                SetTimer(self.dock.hwnd(), TIMER_PREVIA_SAIR, 250, None);
            }
        }
    }

    /// Liga/desliga o modo teclado do dock: as setas, Enter e Esc só são
    /// capturadas enquanto ele está ativo.
    fn modo_teclado_dock(&mut self, ligar: bool) {
        let h = self.topo.hwnd();
        unsafe {
            for (id, vk) in TECLAS_DOCK {
                let _ = UnregisterHotKey(h, id);
                if ligar {
                    let _ = RegisterHotKey(h, id, MOD_NOREPEAT, vk);
                }
            }
        }
        self.dock.definir_foco(if ligar { Some(0) } else { None });
    }

    fn definir_posicao_do_dock(&mut self, p: dock::Posicao) {
        self.config.dock_posicao = match p {
            dock::Posicao::Esquerda => "esquerda",
            dock::Posicao::Direita => "direita",
            dock::Posicao::Baixo => "baixo",
        }
        .into();
        self.config.salvar();
        self.fechar_previa();
        self.dock.definir_posicao(&self.gpu, p);
        self.dock.sujo = true;
    }

    fn fechar_previa(&mut self) {
        if self.previa.take().is_some() {
            unsafe {
                let _ = KillTimer(self.dock.hwnd(), TIMER_PREVIA_SAIR);
            }
        }
    }

    /// Começa o gênio de ida (minimizou) ou de volta (clique na telinha).
    fn comecar_genio(&mut self, i: usize, h: HWND, volta: Option<genio::Ret>) {
        let Some(l) = self.laterais.get_mut(i) else { return };
        self.genios.retain(|g| g.hwnd_fonte != h);
        let para = match volta {
            Some(r) => Some(r),
            None => l.esperar_genio(h),
        };
        let de = genio::retangulo_da_janela(h);
        let g = match (de, para) {
            (Some(de), Some(para)) => {
                let palco = self.palco.take().or_else(|| genio::Palco::new(&self.gpu));
                palco.and_then(|p| genio::Genio::new(p, h, de, para, volta.is_some()))
            }
            _ => None,
        };
        match g {
            Some(mut g) => {
                if g.volta {
                    // O cartão some antes da janela voltar (a miniatura dele
                    // é ao vivo e mostraria o app se redesenhando).
                    let mut escondeu = false;
                    for l in &mut self.laterais {
                        escondeu |= l.esconder_cartao(h);
                    }
                    if escondeu {
                        unsafe {
                            let _ = windows::Win32::Graphics::Dwm::DwmFlush();
                        }
                    }
                    g.escondida = genio::restaurar_invisivel(h);
                }
                self.genios.push(g)
            }
            None if volta.is_some() => {
                restaurar_janela(h);
            }
            None => l.revelar(h),
        }
    }

    fn lembrar_janelas(&self) {
        let v = self.dock.janelas_conhecidas();
        CONHECIDAS.with(|c| *c.borrow_mut() = v.into_iter().collect());
    }

    fn precisa_desenhar(&self) -> bool {
        self.opcoes.estresse
            || self.dock.sujo
            || self.dock.animando()
            || self.medidor.sujo
            || self.topo.sujo
            || self.central.as_ref().map_or(false, |c| c.sujo)
            || self.busca.as_ref().map_or(false, |b| b.sujo && b.visivel)
            || self.launchpad.as_ref().map_or(false, |l| l.sujo || l.animando())
            || self.laterais.iter().any(|l| l.sujo || l.animando())
            || self.topos_extra.iter().any(|t| t.sujo)
            || self.preferencias.as_ref().map_or(false, |p| p.sujo)
            || self.painel_monitor.as_ref().map_or(false, |p| p.sujo && p.visivel)
            || self.calendario.as_ref().map_or(false, |c| c.sujo && c.visivel)
            || self.finders.iter().any(|f| f.sujo || f.animando())
            || !self.genios.is_empty()
            || self.visao.as_ref().map_or(false, |v| v.sujo || v.animando() || v.acabou)
            || self.alternador.as_ref().map_or(false, |a| a.sujo && a.visivel)
            || self.previa.as_ref().map_or(false, |p| p.sujo)
    }

    // ----- Finder -----

    /// Ícone do Finder no dock: traz a janela aberta, ou abre uma nova.
    fn finder_do_dock(&mut self) {
        if let Some(f) = self.finders.last() {
            f.trazer();
        } else {
            self.novo_finder(None);
        }
    }

    fn novo_finder(&mut self, busca: Option<String>) {
        if let Ok(mut f) = finder::Finder::abrir(&self.gpu, finder::pasta_inicial()) {
            if let Some(t) = busca {
                f.buscar_por(&t);
            }
            self.finders.push(f);
        }
    }

    fn mensagem_finder(&mut self, i: usize, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let f = &mut self.finders[i];
        let r = f.mensagem(&self.gpu, msg, wp, lp);
        if std::mem::take(&mut f.mudou_minimizada) {
            // Mesmo caminho das janelas dos outros apps (Palco, gênio, dock).
            let min = f.minimizada as isize;
            unsafe {
                let _ = PostMessageW(self.dock.hwnd(), WM_APP_MINIMIZACAO, WPARAM(f.hwnd().0 as usize), LPARAM(min));
            }
        }
        let f = &mut self.finders[i];
        let (fechar, nova) = (f.fechar, std::mem::take(&mut f.nova_janela));
        if fechar {
            // Destrói fora da mensagem da própria janela.
            unsafe {
                let _ = PostMessageW(self.topo.hwnd(), WM_APP_FECHAR_FINDER, WPARAM(0), LPARAM(0));
            }
            let _ = unsafe { ShowWindowAsync(f.hwnd(), SW_HIDE) };
        }
        if nova {
            self.novo_finder(None);
        }
        r
    }

    /// WM_CLOSE (Alt+F4, taskkill sem /F): nos painéis fecha só o painel;
    /// em qualquer outra janela do Cais, o Cais sai limpo (devolvendo a
    /// barra de tarefas). Nunca deixa o Windows destruir uma janela solta.
    fn pedido_de_fechar(&mut self, hwnd: HWND) -> LRESULT {
        if self.visao.as_ref().is_some_and(|v| v.hwnds().contains(&hwnd)) {
            if let Some(v) = &mut self.visao {
                v.fechar(None);
            }
        } else if self.alternador.as_ref().map(|a| a.hwnd()) == Some(hwnd) {
            self.alternador = None;
        } else if self.previa.as_ref().map(|p| p.hwnd()) == Some(hwnd) {
            self.fechar_previa();
        } else if self.preferencias.as_ref().map(|p| p.hwnd()) == Some(hwnd) {
            self.preferencias = None;
        } else if let Some(f) = self.finders.iter_mut().find(|f| f.hwnd() == hwnd) {
            f.fechar = true;
            unsafe {
                let _ = PostMessageW(self.topo.hwnd(), WM_APP_FECHAR_FINDER, WPARAM(0), LPARAM(0));
            }
        } else if let Some(p) = self.painel_monitor.as_mut().filter(|p| p.hwnd() == hwnd) {
            p.fechar();
            self.agendar_monitor();
        } else if let Some(c) = self.calendario.as_mut().filter(|c| c.hwnd() == hwnd) {
            c.fechar();
        } else if self.central.as_ref().map(|c| c.hwnd()) == Some(hwnd) {
            self.fechar_central();
        } else if let Some(b) = self.busca.as_mut().filter(|b| b.hwnd() == hwnd) {
            b.fechar();
        } else if let Some(l) = self.launchpad.as_mut().filter(|l| l.hwnd() == hwnd) {
            l.fechar = true;
            unsafe {
                let _ = PostMessageW(self.topo.hwnd(), WM_APP_FECHAR_LAUNCHPAD, WPARAM(0), LPARAM(0));
            }
        } else {
            unsafe { PostQuitMessage(0) };
        }
        LRESULT(0)
    }

    // ----- vários monitores -----

    /// Barra do topo número i (0 = monitor principal).
    fn topo_n(&mut self, i: usize) -> &mut topo::Topo {
        if i == 0 || i > self.topos_extra.len() {
            &mut self.topo
        } else {
            &mut self.topos_extra[i - 1]
        }
    }

    /// Cria (ou refaz, se os monitores mudaram) a barra do topo e a faixa de
    /// telinhas de cada monitor. O dock fica só no principal.
    fn atualizar_monitores(&mut self) {
        let lista = janela::monitores();
        let ids: Vec<isize> = lista.iter().map(|m| m.0 as isize).collect();
        if ids == self.monitores_conhecidos {
            return;
        }
        self.monitores_conhecidos = ids;
        self.fechar_central();
        // Desfaz os dos monitores secundários (e as faixas, que serão refeitas).
        for t in self.topos_extra.drain(..) {
            t.sair_do_windows();
        }
        for l in self.laterais.drain(..) {
            l.sair_do_windows();
        }
        let estado = self.sistema.ler();
        let foco = unsafe { GetForegroundWindow() };
        for (k, &m) in lista.iter().enumerate() {
            if let Ok(mut l) = lateral::Lateral::new(&self.gpu, true, self.config.opcoes_lateral(), m) {
                if self.config.minimizar_para == "lateral" {
                    l.ler_minimizadas(&self.gpu);
                }
                self.laterais.push(l);
            }
            if k == 0 {
                continue; // a barra do principal já existe
            }
            if let Ok(mut t) = topo::Topo::new(&self.gpu, true, m) {
                t.definir_sistema(&estado);
                t.primeiro_plano_mudou(foco);
                let _ = t.desenhar(&self.gpu);
                t.mostrar();
                self.topos_extra.push(t);
            }
        }
        let _ = self.topo.reposicionar(&self.gpu);
        let _ = self.dock.reposicionar(&self.gpu);
    }

    fn opcoes_nas_laterais(&mut self) {
        let mut o = self.config.opcoes_lateral();
        o.genio = o.genio && !self.movimento_reduzido();
        for l in &mut self.laterais {
            l.definir_opcoes(&self.gpu, o);
        }
    }

    /// Mensagens das barras do topo dos monitores secundários.
    fn mensagem_topo_extra(&mut self, i: usize, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let n = i + 1;
        match msg {
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE)),
            dock::WM_APPBAR => {
                let t = &mut self.topos_extra[i];
                match wp.0 {
                    ABN_FULLSCREENAPP => unsafe {
                        let modo = if lp.0 != 0 { SW_HIDE } else { SW_SHOWNOACTIVATE };
                        let _ = ShowWindow(t.hwnd(), modo);
                    },
                    ABN_POSCHANGED => {
                        let _ = t.reposicionar(&self.gpu);
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let _ = self.topos_extra[i].reposicionar(&self.gpu);
                Some(LRESULT(0))
            }
            WM_CLOSE => Some(LRESULT(0)),
            WM_LBUTTONDOWN | WM_RBUTTONUP => {
                if msg == WM_RBUTTONUP || !self.topos_extra[i].clique_na_central(lp) {
                    self.fechar_central();
                }
                let r = self.topos_extra[i].mensagem(&self.gpu, msg, wp, lp);
                self.pedidos_de(n);
                r
            }
            _ => self.topos_extra[i].mensagem(&self.gpu, msg, wp, lp),
        }
    }

    /// O que a barra i pediu (Central, busca, Preferências, reiniciar, sair).
    // ----- monitor do sistema -----

    /// Liga, troca o ritmo ou desliga o timer do monitor: 1 s com o painel
    /// aberto, 2 s só com o texto na barra, nenhum timer se nada aparece.
    fn agendar_monitor(&mut self) {
        let painel = self.painel_monitor.as_ref().map_or(false, |p| p.visivel);
        let barra = self.config.monitor_na_barra;
        let h = self.topo.hwnd();
        unsafe {
            if painel || barra {
                SetTimer(h, monitor::TIMER_MONITOR, if painel { 1000 } else { 2000 }, None);
            } else {
                let _ = KillTimer(h, monitor::TIMER_MONITOR);
            }
        }
        if barra {
            self.tick_monitor();
        } else {
            self.topo.definir_monitor(None);
            for t in &mut self.topos_extra {
                t.definir_monitor(None);
            }
        }
    }

    fn tick_monitor(&mut self) {
        let l = self.leitor.ler();
        if self.config.monitor_na_barra {
            let texto = monitor::texto_da_barra(&l);
            self.topo.definir_monitor(Some(texto.clone()));
            for t in &mut self.topos_extra {
                t.definir_monitor(Some(texto.clone()));
            }
        }
        if let Some(p) = self.painel_monitor.as_mut().filter(|p| p.visivel) {
            p.atualizar(l);
        }
    }

    fn alternar_monitor(&mut self, i: usize) {
        if let Some(p) = &mut self.painel_monitor {
            if p.visivel {
                p.fechar();
                self.agendar_monitor();
                return;
            }
            if p.acabou_de_fechar() {
                return;
            }
        }
        self.fechar_central();
        if self.painel_monitor.is_none() {
            match monitor::Painel::new(&self.gpu) {
                Ok(p) => self.painel_monitor = Some(p),
                Err(_) => return,
            }
        }
        // Primeira leitura de CPU precisa de duas amostras: lê agora para
        // "zerar" e o timer de 1 s traz o primeiro valor real.
        let l = self.leitor.ler();
        let (direita, baixo) = self.topo_n(i).canto();
        self.monitor_dono = i;
        if let Some(p) = &mut self.painel_monitor {
            p.abrir(&self.gpu, l, direita, baixo);
        }
        self.agendar_monitor();
    }

    /// Clique no relógio: abre ou fecha o calendário embaixo dele.
    fn alternar_calendario(&mut self, i: usize) {
        if let Some(c) = &mut self.calendario {
            if c.visivel {
                c.fechar();
                return;
            }
            if c.acabou_de_fechar() {
                return; // o clique que fechou foi no próprio relógio
            }
        }
        self.fechar_central();
        if self.calendario.is_none() {
            match calendario::Calendario::new(&self.gpu) {
                Ok(c) => self.calendario = Some(c),
                Err(_) => return,
            }
        }
        let (direita, baixo) = self.topo_n(i).canto();
        if let Some(c) = &mut self.calendario {
            c.abrir(&self.gpu, direita, baixo);
        }
    }

    /// A lista de ícones da bandeja mudou: bitmaps novos para as barras.
    fn atualizar_bandeja(&mut self) {
        self.bandeja_icones = bandeja::icones();
        let px = (topo::ICONE_BANDEJA * self.dock.escala()).round().max(16.0) as u32;
        let mut usados = std::collections::HashSet::new();
        let mut v = Vec::with_capacity(self.bandeja_icones.len());
        for i in &self.bandeja_icones {
            let chave = i.hicon.0 as isize;
            usados.insert(chave);
            let b = self
                .bandeja_bitmaps
                .entry(chave)
                .or_insert_with(|| apps::bitmap_de_hicon(&self.gpu.d2d_dc, i.hicon, px))
                .clone();
            v.push(b);
        }
        self.bandeja_bitmaps.retain(|k, _| usados.contains(k));
        let dicas: Vec<String> = self.bandeja_icones.iter().map(|i| i.dica.clone()).collect();
        self.topo.definir_bandeja(v.clone(), dicas.clone());
        for t in &mut self.topos_extra {
            t.definir_bandeja(v.clone(), dicas.clone());
        }
    }

    fn pedidos_de(&mut self, i: usize) {
        if let Some((k, direito, x, y)) = self.topo_n(i).pedido_bandeja.take() {
            if let Some(ic) = self.bandeja_icones.get(k).cloned() {
                // Dois cliques seguidos no mesmo ícone = clique duplo.
                let duplo = !direito
                    && self.ultimo_clique_bandeja.is_some_and(|(j, t)| {
                        j == k && t.elapsed().as_millis() < unsafe { GetDoubleClickTime() } as u128
                    });
                self.ultimo_clique_bandeja = (!duplo && !direito).then(|| (k, Instant::now()));
                bandeja::clicar(&ic, direito, duplo, x, y);
            }
        }
        if std::mem::take(&mut self.topo_n(i).pedido_central) {
            self.alternar_central(i);
        }
        if std::mem::take(&mut self.topo_n(i).pedido_busca) {
            self.alternar_busca();
        }
        if std::mem::take(&mut self.topo_n(i).pedido_calendario) {
            self.alternar_calendario(i);
        }
        if let Some(k) = self.topo_n(i).pedido_area.take() {
            areas::ir_para(self.areas.atual, k);
        }
        match self.topo_n(i).pedido_cais.take() {
            Some(topo::PedidoCais::Preferencias) => self.abrir_preferencias(),
            Some(topo::PedidoCais::Monitor) => self.alternar_monitor(i),
            Some(topo::PedidoCais::Reiniciar) => reiniciar(),
            Some(topo::PedidoCais::Sair) => unsafe { PostQuitMessage(0) },
            None => {}
        }
    }


    // ----- Preferências -----

    fn valores(&self) -> preferencias::Valores {
        let c = &self.config;
        preferencias::Valores {
            esconder_barra: c.esconder_barra,
            iniciar_com_windows: config::inicia_com_windows(),
            ocultar_auto: c.ocultar_auto,
            ampliar: c.ampliar,
            minimizar: modo_de(&c.minimizar_para) as usize,
            inclinar: c.lateral_inclinar,
            reservar: c.lateral_reservar,
            maximo: c.lateral_maximo,
            genio: c.efeito_genio,
            instalado_aqui: instalador::rodando_instalado(),
            tamanho_lateral: preferencias::TAMANHOS.iter().position(|t| t.1 == c.lateral_tamanho).unwrap_or(2),
            atalho: preferencias::ATALHOS
                .iter()
                .position(|a| a.1 == c.atalho_busca)
                .unwrap_or(0),
            atalho_ok: self.atalho_ok || c.atalho_busca == "nenhum",
            pasta: c.pasta().to_string_lossy().into_owned(),
            monitor_na_barra: c.monitor_na_barra,
            layouts: c.layouts,
            layout_principal: c.layout_principal,
            layout_outros: c.layout_outros,
            layout_espaco: c.layout_espaco,
            aparencia: tema::indice(&tema::APARENCIAS.map(|a| a.0), &c.aparencia),
            destaque: tema::indice(&tema::DESTAQUES.map(|a| a.0), &c.destaque),
            transparencia: tema::indice(&tema::TRANSPARENCIAS.map(|a| a.0), &c.transparencia),
            pastas_no_finder: c.pastas_no_finder,
            idioma: idioma::IDIOMAS.iter().position(|a| a.1 == c.idioma).unwrap_or(0),
            cantos: [0, 1, 2, 3].map(|k| cantos::indice(&c.cantos[k])),
            atalho_visao: preferencias::ATALHOS_VISAO.iter().position(|a| a.1 == c.atalho_visao).unwrap_or(0),
            alternador: c.alternador,
            dock_posicao: preferencias::POSICOES.iter().position(|a| a.1 == c.dock_posicao).unwrap_or(0),
            previas: c.previas_no_dock,
            reduzir: preferencias::REDUZIR.iter().position(|a| a.1 == c.reduzir_movimento).unwrap_or(0),
            admin: instalador::admin_ligado(),
            menus_electron: c.menus_electron,
            icones_uniformes: c.icones_uniformes,
            arredondar_janelas: c.arredondar_janelas,
        }
    }

    /// O tema mudou (Preferências ou o Windows): redesenha tudo.
    fn tema_mudou(&mut self) {
        janela::menus_no_tema();
        self.dock.sujo = true;
        self.topo.sujo = true;
        for t in &mut self.topos_extra {
            t.sujo = true;
        }
        for l in &mut self.laterais {
            l.sujo = true;
        }
        for f in &mut self.finders {
            f.sujo = true;
        }
        if let Some(c) = &mut self.central {
            c.sujo = true;
        }
        if let Some(b) = &mut self.busca {
            b.sujo = true;
        }
        if let Some(l) = &mut self.launchpad {
            l.sujo = true;
        }
        if let Some(p) = &mut self.preferencias {
            p.sujo = true;
        }
        if let Some(p) = &mut self.painel_monitor {
            p.sujo = true;
        }
        if let Some(c) = &mut self.calendario {
            c.sujo = true;
        }
    }

    fn abrir_preferencias(&mut self) {
        self.fechar_central();
        if let Some(p) = &self.preferencias {
            p.trazer();
            return;
        }
        let v = self.valores();
        if let Ok(p) = preferencias::Preferencias::abrir(&self.gpu, v) {
            self.preferencias = Some(p);
        }
    }

    fn mensagem_preferencias(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if msg == WM_CLOSE {
            self.preferencias = None; // fecha só esta janela
            return Some(LRESULT(0));
        }
        let p = self.preferencias.as_mut()?;
        let r = p.mensagem(msg, wp, lp);
        let mudancas = std::mem::take(&mut p.mudancas);
        if !mudancas.is_empty() {
            for m in mudancas {
                self.aplicar(m);
            }
            self.config.salvar();
            let v = self.valores();
            if let Some(p) = &mut self.preferencias {
                p.definir(v);
            }
        }
        r
    }

    /// Aplica uma mudança feita na tela de Preferências.
    fn aplicar(&mut self, m: preferencias::Mudanca) {
        use preferencias::{Mudanca, Opcao};
        match m {
            Mudanca::Alternar(Opcao::EsconderBarra) => self.alternar_barra(),
            Mudanca::Alternar(Opcao::IniciarComWindows) => {
                config::definir_inicio_com_windows(!config::inicia_com_windows())
            }
            Mudanca::Alternar(Opcao::OcultarAuto) => self.alternar_auto(),
            Mudanca::Alternar(Opcao::Ampliar) => {
                self.config.ampliar = !self.config.ampliar;
                self.dock.ampliacao = if self.config.ampliar { dock::AMPLIACAO } else { 1.0 };
                self.dock.sujo = true;
            }
            Mudanca::Alternar(Opcao::Genio) => {
                self.config.efeito_genio = !self.config.efeito_genio;
                self.opcoes_nas_laterais();
                animacao_do_windows(self.usar_genio());
            }
            Mudanca::Alternar(Opcao::Inclinar) => {
                self.config.lateral_inclinar = !self.config.lateral_inclinar;
                self.opcoes_nas_laterais();
            }
            Mudanca::Alternar(Opcao::Reservar) => {
                self.config.lateral_reservar = !self.config.lateral_reservar;
                self.opcoes_nas_laterais();
            }
            Mudanca::Escolher(Opcao::Minimizar, k) => self.definir_minimizar(k as u8),
            Mudanca::Escolher(Opcao::TamanhoLateral, k) => {
                self.config.lateral_tamanho = preferencias::TAMANHOS[k.min(2)].1.to_string();
                self.opcoes_nas_laterais();
            }
            Mudanca::Escolher(Opcao::Maximo, k) => {
                self.config.lateral_maximo = k.clamp(1, 8);
                self.opcoes_nas_laterais();
            }
            Mudanca::Escolher(Opcao::Atalho, k) => {
                let novo = preferencias::ATALHOS[k.min(3)].1;
                self.config.atalho_busca = novo.to_string();
                unsafe {
                    let _ = UnregisterHotKey(self.topo.hwnd(), ATALHO_BUSCA);
                }
                self.atalho_ok = registrar_atalho(self.topo.hwnd(), novo);
            }
            Mudanca::Alternar(Opcao::MonitorNaBarra) => {
                self.config.monitor_na_barra = !self.config.monitor_na_barra;
                self.agendar_monitor();
            }
            Mudanca::Alternar(Opcao::Layouts) => {
                self.config.layouts = !self.config.layouts;
                self.config.aplicar_layouts();
            }
            Mudanca::Escolher(Opcao::LayoutPrincipal, k) => {
                self.config.layout_principal = k.min(layouts::LAYOUTS.len() - 1);
                self.config.aplicar_layouts();
            }
            Mudanca::Escolher(Opcao::LayoutOutros, k) => {
                self.config.layout_outros = k.min(layouts::LAYOUTS.len() - 1);
                self.config.aplicar_layouts();
            }
            Mudanca::Escolher(Opcao::LayoutEspaco, k) => {
                self.config.layout_espaco = k.min(layouts::ESPACOS.len() - 1);
                self.config.aplicar_layouts();
            }
            Mudanca::Acao(Opcao::Organizar) => {
                // Funciona mesmo com os layouts desligados (é um pedido seu).
                let ligado = self.config.layouts;
                self.config.layouts = true;
                self.config.aplicar_layouts();
                layouts::organizar_todos();
                self.config.layouts = ligado;
                self.config.aplicar_layouts();
            }
            Mudanca::Escolher(Opcao::Aparencia, k) => {
                self.config.aparencia = tema::APARENCIAS[k.min(tema::APARENCIAS.len() - 1)].0.to_string();
                if self.config.aplicar_tema() {
                    self.tema_mudou();
                }
            }
            Mudanca::Escolher(Opcao::Destaque, k) => {
                self.config.destaque = tema::DESTAQUES[k.min(tema::DESTAQUES.len() - 1)].0.to_string();
                if self.config.aplicar_tema() {
                    self.tema_mudou();
                }
            }
            Mudanca::Escolher(Opcao::Transparencia, k) => {
                self.config.transparencia = tema::TRANSPARENCIAS[k.min(tema::TRANSPARENCIAS.len() - 1)].0.to_string();
                if self.config.aplicar_tema() {
                    self.tema_mudou();
                }
            }
            Mudanca::Alternar(Opcao::PastasNoFinder) => {
                self.config.pastas_no_finder = !self.config.pastas_no_finder;
                pastas::ligar(self.config.pastas_no_finder);
            }
            Mudanca::Acao(Opcao::Reiniciar) => reiniciar(),
            Mudanca::Acao(Opcao::Instalar) => match instalador::instalar() {
                Ok(exe) => {
                    if let Ok(mut g) = REABRIR_EM.lock() {
                        *g = Some(exe);
                    }
                    reiniciar(); // reabre já da pasta de programas
                }
                Err(e) => unsafe {
                    MessageBoxW(None, &windows::core::HSTRING::from(e), windows::core::w!("Cais"), MB_OK | MB_ICONWARNING);
                },
            },
            Mudanca::Acao(Opcao::Desinstalar) => {
                if let Some(apagar) = instalador::confirmar_desinstalar() {
                    if let Ok(mut g) = DESINSTALAR_AO_SAIR.lock() {
                        *g = Some(apagar);
                    }
                    unsafe { PostQuitMessage(0) };
                }
            }
            Mudanca::Acao(Opcao::Sair) => unsafe { PostQuitMessage(0) },
            Mudanca::Acao(Opcao::AbrirPasta) => apps::abrir(&self.config.pasta().to_string_lossy()),
            Mudanca::Acao(Opcao::PorLaunchpad) => {
                self.dock.modelo.adicionar_launchpad();
                self.dock.atualizar_apps(&self.gpu);
            }
            Mudanca::Escolher(Opcao::Idioma, k) => {
                self.config.idioma = idioma::IDIOMAS[k.min(2)].1.to_string();
                if idioma::configurar(&self.config.idioma) {
                    self.idioma_mudou();
                }
            }
            Mudanca::Escolher(Opcao::AtalhoVisao, k) => {
                self.config.atalho_visao = preferencias::ATALHOS_VISAO[k.min(2)].1.to_string();
                registrar_atalho_id(self.topo.hwnd(), ATALHO_VISAO, &self.config.atalho_visao);
            }
            Mudanca::Alternar(Opcao::Alternador) => {
                self.config.alternador = !self.config.alternador;
                self.ligar_entrada();
            }
            Mudanca::Escolher(Opcao::Canto(c), k) => {
                self.config.cantos[(c as usize).min(3)] = cantos::ACOES[k.min(cantos::ACOES.len() - 1)].0.to_string();
                self.ligar_entrada();
            }
            Mudanca::Escolher(Opcao::DockPosicao, k) => {
                self.definir_posicao_do_dock(dock::Posicao::de(preferencias::POSICOES[k.min(2)].1));
            }
            Mudanca::Alternar(Opcao::Previas) => {
                self.config.previas_no_dock = !self.config.previas_no_dock;
                if !self.config.previas_no_dock {
                    self.fechar_previa();
                }
            }
            Mudanca::Escolher(Opcao::ReduzirMovimento, k) => {
                self.config.reduzir_movimento = preferencias::REDUZIR[k.min(2)].1.to_string();
                animacao_do_windows(self.usar_genio());
                self.opcoes_nas_laterais();
            }
            Mudanca::Acao(Opcao::Admin) => instalador::alternar_admin(self.dock.hwnd()),
            Mudanca::Alternar(Opcao::ArredondarJanelas) => {
                self.config.arredondar_janelas = !self.config.arredondar_janelas;
                arredondar::ligar(self.config.arredondar_janelas);
            }
            Mudanca::Alternar(Opcao::IconesUniformes) => {
                self.config.icones_uniformes = !self.config.icones_uniformes;
                placa::ligar(self.config.icones_uniformes);
                // Recarrega os ícones (o cache em disco guarda os originais).
                self.dock.modelo.esquecer_icones();
                self.dock.atualizar_apps(&self.gpu);
                self.catalogo.soltar_icones();
                for l in &mut self.laterais {
                    l.esquecer_icones();
                }
                self.dock.sujo = true;
            }
            Mudanca::Alternar(Opcao::MenusElectron) => {
                self.config.menus_electron = !self.config.menus_electron;
                menus_uia::ler_electron(self.config.menus_electron);
            }
            Mudanca::Acao(Opcao::AbrirVisao) => {
                // Fecha as Preferências (fora da mensagem delas) para não
                // esconderem as janelas.
                if let Some(p) = &self.preferencias {
                    unsafe {
                        let _ = PostMessageW(p.hwnd(), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
                self.alternar_visao();
            }
            _ => {}
        }
    }

    /// O idioma mudou: refaz o que tem texto guardado (nomes do dock) e
    /// redesenha tudo.
    fn idioma_mudou(&mut self) {
        self.dock.modelo.renomear_especiais();
        self.dock.atualizar_apps(&self.gpu);
        self.tema_mudou();
    }

    /// Menu do dock: para onde vão as janelas minimizadas.
    fn definir_minimizar(&mut self, modo: u8) {
        let nome = ["lateral", "dock", "nao"][modo.min(2) as usize];
        self.config.minimizar_para = nome.to_string();
        self.config.salvar();
        self.dock.modo_minimizar = modo;
        self.dock.definir_minimizar_no_dock(nome == "dock");
        for l in &mut self.laterais {
            if nome == "lateral" {
                l.ler_minimizadas(&self.gpu);
            } else {
                l.limpar(&self.gpu);
            }
        }
        animacao_do_windows(self.usar_genio());
    }

    fn mensagem_lateral(&mut self, i: usize, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let gpu = &self.gpu;
        let l = &mut self.laterais[i];
        match msg {
            dock::WM_APPBAR => {
                match wp.0 {
                    ABN_FULLSCREENAPP => l.definir_tela_cheia(gpu, lp.0 != 0),
                    ABN_POSCHANGED => l.reposicionar(gpu),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                l.reposicionar(gpu);
                Some(LRESULT(0))
            }
            _ => {
                let r = l.mensagem(gpu, msg, wp, lp);
                if let Some((h, para)) = l.pedido_volta.take() {
                    self.comecar_genio(i, h, Some(para));
                }
                r
            }
        }
    }

    // ----- busca e Launchpad -----

    fn alternar_busca(&mut self) {
        if let Some(b) = &mut self.busca {
            if b.visivel {
                b.fechar();
                return;
            }
            if b.acabou_de_fechar() {
                return;
            }
        }
        self.fechar_central();
        self.fechar_launchpad();
        if self.busca.is_none() {
            match busca::Busca::new(&self.gpu) {
                Ok(b) => self.busca = Some(b),
                Err(_) => return,
            }
        }
        if let Some(b) = &mut self.busca {
            b.abrir(&self.gpu, &mut self.catalogo);
        }
    }

    fn alternar_launchpad(&mut self) {
        if self.launchpad.is_some() {
            self.fechar_launchpad();
            return;
        }
        self.fechar_central();
        if let Some(b) = &mut self.busca {
            b.fechar();
        }
        if let Ok(l) = launchpad::Launchpad::abrir(&self.gpu, &mut self.catalogo) {
            let h = l.hwnd();
            self.launchpad = Some(l);
            // O dock continua por cima do Launchpad, como no Mac: o
            // Launchpad entra na pilha logo abaixo do dock.
            unsafe {
                let _ = SetWindowPos(
                    h,
                    self.dock.hwnd(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
    }

    fn fechar_launchpad(&mut self) {
        if self.launchpad.take().is_some() {
            self.soltar_icones_se_tudo_fechado();
        }
    }

    fn soltar_icones_se_tudo_fechado(&mut self) {
        let busca_aberta = self.busca.as_ref().map_or(false, |b| b.visivel);
        if !busca_aberta && self.launchpad.is_none() {
            self.catalogo.soltar_icones();
        }
    }

    fn mensagem_busca(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let b = self.busca.as_mut()?;
        let r = b.mensagem(&self.gpu, &mut self.catalogo, msg, wp, lp);
        if let Some(t) = b.pedido_arquivos.take() {
            self.novo_finder(Some(t));
        }
        let b = self.busca.as_mut()?;
        if !b.visivel {
            self.soltar_icones_se_tudo_fechado();
        }
        r
    }

    fn mensagem_launchpad(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let l = self.launchpad.as_mut()?;
        let r = l.mensagem(&mut self.catalogo, msg, wp, lp);
        if l.fechar {
            // Sai da pilha da mensagem antes de destruir a janela.
            unsafe {
                let _ = PostMessageW(self.topo.hwnd(), WM_APP_FECHAR_LAUNCHPAD, WPARAM(0), LPARAM(0));
            }
            let _ = unsafe { ShowWindowAsync(l.hwnd(), SW_HIDE) };
        }
        r
    }

    fn central_visivel(&self) -> bool {
        self.central.as_ref().map_or(false, |c| c.visivel)
    }

    fn fechar_central(&mut self) {
        if let Some(c) = &mut self.central {
            if c.visivel {
                c.fechar();
                let dono = self.central_dono;
                let t = self.topo_n(dono);
                t.central_aberta = false;
                t.sujo = true;
            }
        }
    }

    /// Abre/fecha a Central embaixo da barra `i` (0 = monitor principal).
    fn alternar_central(&mut self, i: usize) {
        if self.central_visivel() {
            self.fechar_central();
            return;
        }
        if self.central.as_ref().map_or(false, |c| c.acabou_de_fechar()) {
            return;
        }
        if self.central.is_none() {
            match central::Central::new(&self.gpu, self.sistema.remetente()) {
                Ok(c) => self.central = Some(c),
                Err(_) => return,
            }
        }
        let estado = self.sistema.ler();
        let (direita, baixo) = self.topo_n(i).canto();
        self.central_dono = i;
        if let Some(c) = &mut self.central {
            c.definir(&self.gpu, &estado);
            c.abrir(&self.gpu, direita, baixo);
        }
        let t = self.topo_n(i);
        t.central_aberta = true;
        t.sujo = true;
    }

    fn mensagem_central(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let c = self.central.as_mut()?;
        let r = c.mensagem(&self.gpu, msg, wp, lp);
        let fechou = !c.visivel;
        let dono = self.central_dono;
        let t = self.topo_n(dono);
        if fechou && t.central_aberta {
            t.central_aberta = false;
            t.sujo = true;
        }
        r
    }

    /// Um quadro. Só é chamado enquanto há algo mudando na tela.
    fn quadro(&mut self) {
        let agora = Instant::now();
        let dt = match self.ultimo_quadro {
            Some(t) => (agora - t).as_secs_f32().min(1.0 / 20.0),
            None => 1.0 / 60.0,
        };
        self.ultimo_quadro = Some(agora);

        if self.opcoes.estresse {
            let t = (agora - self.inicio).as_secs_f32();
            let l = self.dock.largura_dip();
            self.dock
                .definir_ponteiro(Some(l * 0.5 + (t * 1.7).sin() * l * 0.12));
        }
        self.dock.passo(dt);
        if self.dock.sujo || self.opcoes.estresse {
            let _ = self.dock.desenhar(&self.gpu);
        }
        if self.medidor.sujo {
            let _ = self.medidor.desenhar(&self.gpu);
        }
        if self.topo.sujo {
            let _ = self.topo.desenhar(&self.gpu);
        }
        if let Some(b) = &mut self.busca {
            if b.sujo && b.visivel {
                let _ = b.desenhar(&self.gpu, &mut self.catalogo);
            }
        }
        if let Some(p) = &mut self.painel_monitor {
            if p.sujo && p.visivel {
                let _ = p.desenhar(&self.gpu);
            }
        }
        if let Some(c) = &mut self.calendario {
            if c.sujo && c.visivel {
                let _ = c.desenhar(&self.gpu);
            }
        }
        for f in &mut self.finders {
            f.passo(dt);
            if f.sujo {
                let _ = f.desenhar(&self.gpu);
            }
        }
        if let Some(p) = &mut self.preferencias {
            if p.sujo {
                let _ = p.desenhar(&self.gpu);
            }
        }
        for l in &mut self.laterais {
            l.passo(dt);
            if l.sujo {
                let _ = l.desenhar(&self.gpu);
            }
        }
        for t in &mut self.topos_extra {
            if t.sujo {
                let _ = t.desenhar(&self.gpu);
            }
        }
        // Efeitos gênio. Ida: a telinha começa a aparecer antes do fim, por
        // baixo do efeito que some (sem quadro vazio). Volta: a janela de
        // verdade aparece e o efeito fica por cima, parado e depois sumindo
        // aos poucos, até ela estar desenhada.
        let mut i = 0;
        while i < self.genios.len() {
            let acabou = self.genios[i].passo();
            let g = &mut self.genios[i];
            let h = g.hwnd_fonte;
            let pronto = if g.volta {
                if acabou && !g.marco {
                    g.marco = true;
                    g.saiu_em = Some(Instant::now());
                    if g.escondida {
                        genio::mostrar(h);
                        unsafe {
                            let _ = SetForegroundWindow(h);
                        }
                    } else if !restaurar_janela(h) {
                        // O Windows não deixou (janela de administrador com
                        // o Cais comum): o cartão volta para o Palco em vez
                        // de a janela sumir.
                        for l in &mut self.laterais {
                            if l.esconder_cartao(h) {
                                l.restaurou(&self.gpu, h);
                                l.minimizou(&self.gpu, h);
                            }
                        }
                    }
                }
                g.saiu_em.is_some_and(|t| t.elapsed().as_millis() >= genio::VOLTA_FIM_MS)
            } else {
                if g.tempo() >= 0.8 && !g.marco {
                    g.marco = true;
                    for l in &mut self.laterais {
                        l.revelar(h);
                    }
                }
                acabou
            };
            if pronto {
                let g = self.genios.remove(i);
                if let Some(p) = g.devolver() {
                    if self.palco.is_none() {
                        self.palco = Some(p);
                    }
                }
            } else {
                i += 1;
            }
        }
        if let Some(l) = &mut self.launchpad {
            l.passo(dt);
            if l.sujo {
                let _ = l.desenhar(&self.gpu, &mut self.catalogo);
            }
        }
        if let Some(v) = &mut self.visao {
            if v.animando() {
                v.passo();
            }
            if v.sujo {
                let _ = v.desenhar(&self.gpu);
            }
            if v.acabou {
                self.visao = None;
            }
        }
        if let Some(a) = &mut self.alternador {
            if a.sujo && a.visivel {
                let _ = a.desenhar(&self.gpu);
            }
        }
        if let Some(p) = &mut self.previa {
            if p.sujo {
                let _ = p.desenhar(&self.gpu);
            }
        }
        if let Some(c) = &mut self.central {
            if c.sujo && c.visivel {
                let _ = c.desenhar(&self.gpu);
            }
            c.sujo = false;
        }
        if !self.precisa_desenhar() {
            self.ultimo_quadro = None;
            self.dock.atualizar_regiao(false);
        }
    }

    fn relatorio(&self) -> String {
        let base = self.base_medicao.expect("medição não iniciada");
        let fim = medidor::amostrar();
        let seg = (fim.instante - base.instante).as_secs_f64();
        let cpu_ms = (fim.cpu_100ns - base.cpu_100ns) as f64 / 10_000.0;
        let quadros = fim.quadros - base.quadros;
        let (ws, privada) = medidor::memoria_mb();
        format!(
            "modo: {}\napps_no_dock: {}\nduracao_s: {seg:.1}\ncpu_ms_total: {cpu_ms:.1}\ncpu_pct_maquina: {:.3}\nnucleos: {}\nquadros: {quadros}\nquadros_por_s: {:.1}\nmemoria_em_uso_mb: {ws:.1}\nmemoria_privada_mb: {privada:.1}\ninicio_ate_primeiro_quadro_ms: {:.1}\nreleituras_de_apps: {}\neventos_destr_nome_cloak_mostra: {:?}\n",
            if self.opcoes.estresse { "estresse (animando sempre)" } else { "parado" },
            self.dock.modelo.itens.len(),
            medidor::cpu_pct(&base, &fim),
            medidor::nucleos(),
            quadros as f64 / seg,
            self.primeiro_quadro_ms,
            RELEITURAS.load(Ordering::Relaxed),
            EVENTOS.iter().map(|e| e.load(Ordering::Relaxed)).collect::<Vec<_>>(),
        )
    }

    fn mensagem_topo(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE)),
            dock::WM_APPBAR => {
                match wp.0 {
                    ABN_FULLSCREENAPP => unsafe {
                        let modo = if lp.0 != 0 { SW_HIDE } else { SW_SHOWNOACTIVATE };
                        let _ = ShowWindow(self.topo.hwnd(), modo);
                    },
                    ABN_POSCHANGED => {
                        let _ = self.topo.reposicionar(&self.gpu);
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let _ = self.topo.reposicionar(&self.gpu);
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            sistema::WM_SISTEMA => {
                let e = self.sistema.ler();
                self.topo.definir_sistema(&e);
                for t in &mut self.topos_extra {
                    t.definir_sistema(&e);
                }
                if let Some(c) = &mut self.central {
                    c.definir(&self.gpu, &e);
                }
                Some(LRESULT(0))
            }
            menus_uia::WM_MENUS_UIA => {
                if let Some((h, titulos)) = menus_uia::pegar() {
                    self.topo.menus_modernos_chegaram(h, &titulos);
                    for t in &mut self.topos_extra {
                        t.menus_modernos_chegaram(h, &titulos);
                    }
                }
                Some(LRESULT(0))
            }
            topo::WM_PRIMEIRO_PLANO => {
                // Outra janela veio para frente: a Central se recolhe.
                let h = HWND(wp.0 as *mut _);
                if self.central_visivel() && self.central.as_ref().map(|c| c.hwnd()) != Some(h) {
                    self.fechar_central();
                }
                // Todas as barras mostram o app em foco.
                for t in &mut self.topos_extra {
                    t.primeiro_plano_mudou(h);
                }
                self.topo.mensagem(&self.gpu, msg, wp, lp)
            }
            WM_LBUTTONDOWN => {
                if !self.topo.clique_na_central(lp) {
                    self.fechar_central();
                }
                let r = self.topo.mensagem(&self.gpu, msg, wp, lp);
                self.pedidos_de(0);
                r
            }
            WM_HOTKEY if wp.0 == ATALHO_BUSCA as usize => {
                self.alternar_busca();
                Some(LRESULT(0))
            }
            WM_HOTKEY if wp.0 == ATALHO_VISAO as usize => {
                self.alternar_visao();
                Some(LRESULT(0))
            }
            WM_HOTKEY if wp.0 == ATALHO_DOCK as usize => {
                self.modo_teclado_dock(self.dock.foco_teclado().is_none());
                Some(LRESULT(0))
            }
            WM_HOTKEY if (10..=15).contains(&(wp.0 as i32)) => {
                let lado = self.dock.posicao != dock::Posicao::Baixo;
                match wp.0 {
                    10 if !lado => self.dock.mover_foco(-1),
                    11 if !lado => self.dock.mover_foco(1),
                    12 if lado => self.dock.mover_foco(-1),
                    13 if lado => self.dock.mover_foco(1),
                    14 => {
                        let f = self.dock.foco_teclado();
                        self.modo_teclado_dock(false);
                        if let Some(i) = f {
                            self.dock.acionar_indice(i);
                            if let Some((pasta, ancora)) = self.dock.pedido_pilha.take() {
                                self.previa = previa::Previa::abrir_pilha(&self.gpu, &pasta, ancora, self.dock.posicao).ok();
                            } else if std::mem::take(&mut self.dock.pedido_finder) {
                                self.finder_do_dock();
                            } else if std::mem::take(&mut self.dock.pedido_launchpad) {
                                self.alternar_launchpad();
                            }
                        }
                    }
                    15 => self.modo_teclado_dock(false),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            areas::WM_AREAS => {
                let antes = self.areas.atual;
                self.areas.ler();
                let (atual, total) = (self.areas.atual, self.areas.total);
                if atual != antes && self.config.minimizar_para == "lateral" {
                    // Cada área de trabalho tem o seu Palco: as janelas das
                    // outras áreas ficam escondidas pelo Windows e saem da faixa.
                    let gpu = &self.gpu;
                    for l in &mut self.laterais {
                        l.limpar(gpu);
                        l.ler_minimizadas(gpu);
                    }
                }
                self.topo.definir_areas(atual, total);
                for t in &mut self.topos_extra {
                    t.definir_areas(atual, total);
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == monitor::TIMER_MONITOR => {
                self.tick_monitor();
                Some(LRESULT(0))
            }
            bandeja::WM_BANDEJA_MUDOU => {
                self.atualizar_bandeja();
                Some(LRESULT(0))
            }
            catalogo::WM_CATALOGO => {
                self.catalogo.receber(lp.0);
                if let Some(b) = &mut self.busca {
                    if b.visivel {
                        b.refazer(&self.gpu, &self.catalogo);
                    }
                }
                if let Some(l) = &mut self.launchpad {
                    if lp.0 == 0 {
                        l.refazer(&self.catalogo);
                    } else {
                        l.sujar();
                    }
                }
                Some(LRESULT(0))
            }
            WM_APP_FECHAR_FINDER => {
                self.finders.retain(|f| !f.fechar);
                Some(LRESULT(0))
            }
            WM_APP_FECHAR_LAUNCHPAD => {
                if self.launchpad.as_ref().map_or(false, |l| l.fechar) {
                    self.fechar_launchpad();
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                self.fechar_central();
                let r = self.topo.mensagem(&self.gpu, msg, wp, lp);
                self.pedidos_de(0);
                r
            }
            _ => self.topo.mensagem(&self.gpu, msg, wp, lp),
        }
    }

    fn mensagem(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if msg == WM_CLOSE {
            return Some(self.pedido_de_fechar(hwnd));
        }
        if hwnd == self.topo.hwnd() {
            return self.mensagem_topo(msg, wp, lp);
        }
        if let Some(v) = self.visao.as_mut().filter(|v| v.hwnds().contains(&hwnd)) {
            if msg == WM_MOUSEACTIVATE {
                return None;
            }
            return v.mensagem(hwnd, msg, wp, lp);
        }
        if let Some(a) = self.alternador.as_mut().filter(|a| a.hwnd() == hwnd) {
            return match msg {
                WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE)),
                WM_LBUTTONUP => {
                    let x = (lp.0 & 0xffff) as i16 as i32;
                    if let Some(h) = a.clique(x) {
                        self.alternador = None;
                        apps::trazer_janela(h);
                    }
                    Some(LRESULT(0))
                }
                _ => None,
            };
        }
        if let Some(p) = self.previa.as_mut().filter(|p| p.hwnd() == hwnd) {
            let r = p.mensagem(msg, wp, lp);
            if p.fechar {
                self.fechar_previa();
            }
            return r;
        }
        if self.central.as_ref().map(|c| c.hwnd()) == Some(hwnd) {
            return self.mensagem_central(msg, wp, lp);
        }
        if self.busca.as_ref().map(|b| b.hwnd()) == Some(hwnd) {
            return self.mensagem_busca(msg, wp, lp);
        }
        if self.launchpad.as_ref().map(|l| l.hwnd()) == Some(hwnd) {
            return self.mensagem_launchpad(msg, wp, lp);
        }
        if self.preferencias.as_ref().map(|p| p.hwnd()) == Some(hwnd) {
            return self.mensagem_preferencias(msg, wp, lp);
        }
        if let Some(i) = self.finders.iter().position(|f| f.hwnd() == hwnd) {
            return self.mensagem_finder(i, msg, wp, lp);
        }
        if let Some(p) = self.painel_monitor.as_mut().filter(|p| p.hwnd() == hwnd) {
            let r = p.mensagem(msg, wp, lp);
            if !p.visivel {
                self.agendar_monitor(); // fechou: o timer volta ao ritmo da barra (ou para)
            }
            return r;
        }
        if let Some(c) = self.calendario.as_mut().filter(|c| c.hwnd() == hwnd) {
            if msg == WM_CLOSE {
                c.fechar();
                return Some(LRESULT(0));
            }
            let r = c.mensagem(msg, wp, lp);
            if std::mem::take(&mut c.pedido_notificacoes) {
                topo::teclas_windows(windows::Win32::UI::Input::KeyboardAndMouse::VK_N);
            }
            return r;
        }
        if let Some(i) = self.laterais.iter().position(|l| l.hwnd() == hwnd) {
            return self.mensagem_lateral(i, msg, wp, lp);
        }
        if let Some(i) = self.topos_extra.iter().position(|t| t.hwnd() == hwnd) {
            return self.mensagem_topo_extra(i, msg, wp, lp);
        }
        if matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN) {
            self.fechar_central(); // clicar no dock também recolhe a Central
        }
        let no_dock = hwnd == self.dock.hwnd();
        let e = self.dock.escala();
        let x = (lp.0 & 0xffff) as i16 as f32 / e;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / e;
        // O dock pensa sempre "deitado"; de lado, as coordenadas giram.
        let (x, y) = if no_dock { self.dock.para_logico(x, y) } else { (x, y) };
        match msg {
            entrada::WM_ALTERNADOR => {
                self.evento_alternador(wp.0);
                Some(LRESULT(0))
            }
            instalador::WM_ADMIN => {
                let v = self.valores();
                if let Some(p) = &mut self.preferencias {
                    p.definir(v);
                }
                Some(LRESULT(0))
            }
            entrada::WM_CANTO => {
                self.canto_pendente = Some(wp.0);
                unsafe {
                    SetTimer(hwnd, TIMER_CANTO, 140, None);
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_CANTO => {
                unsafe {
                    let _ = KillTimer(hwnd, TIMER_CANTO);
                }
                if let Some(c) = self.canto_pendente.take() {
                    self.acao_do_canto(c);
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == alternador::TIMER_ALTERNADOR => {
                unsafe {
                    let _ = KillTimer(hwnd, alternador::TIMER_ALTERNADOR);
                }
                if let Some(a) = &mut self.alternador {
                    a.mostrar();
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_PREVIA => {
                unsafe {
                    let _ = KillTimer(hwnd, TIMER_PREVIA);
                }
                if self.config.previas_no_dock {
                    self.abrir_previa();
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_PREVIA_SAIR => {
                // Fecha a prévia quando o mouse não está nela nem no ícone.
                let mut p = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut p);
                }
                let na_previa = self.previa.as_ref().is_some_and(|v| v.contem(p));
                if !na_previa && !self.dock.mouse_no_item_da_previa() {
                    self.fechar_previa();
                }
                Some(LRESULT(0))
            }
            // Clicar no dock nunca rouba o foco da janela em que você está.
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE)),
            WM_MOUSEMOVE if no_dock => {
                if !self.rastreando_mouse {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                    self.rastreando_mouse = true;
                }
                let antes = self.dock.item_sob_mouse();
                self.dock.mover(x, y);
                let depois = self.dock.item_sob_mouse();
                if antes != depois {
                    unsafe {
                        let _ = KillTimer(hwnd, TIMER_PREVIA);
                        if depois.is_some() && self.config.previas_no_dock {
                            SetTimer(hwnd, TIMER_PREVIA, if self.previa.is_some() { 60 } else { 450 }, None);
                        }
                    }
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE if no_dock => {
                self.rastreando_mouse = false;
                self.dock.sair();
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN if no_dock => {
                self.fechar_previa();
                unsafe {
                    let _ = KillTimer(hwnd, TIMER_PREVIA);
                }
                self.dock.botao_baixo(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP if no_dock => {
                self.dock.botao_cima(x, y);
                if let Some((pasta, ancora)) = self.dock.pedido_pilha.take() {
                    self.fechar_previa();
                    self.previa = previa::Previa::abrir_pilha(&self.gpu, &pasta, ancora, self.dock.posicao).ok();
                    if self.previa.is_some() {
                        unsafe {
                            SetTimer(hwnd, TIMER_PREVIA_SAIR, 250, None);
                        }
                    }
                } else if std::mem::take(&mut self.dock.pedido_finder) {
                    self.fechar_launchpad();
                    self.finder_do_dock();
                } else if std::mem::take(&mut self.dock.pedido_launchpad) {
                    self.alternar_launchpad();
                } else if self.launchpad.is_some() {
                    self.fechar_launchpad(); // abriu outro app pelo dock
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP if no_dock => {
                self.rastreando_mouse = false;
                self.fechar_previa();
                match self.dock.menu(x, y) {
                    dock::AcaoMenu::Sair => unsafe { PostQuitMessage(0) },
                    dock::AcaoMenu::AlternarAuto => self.alternar_auto(),
                    dock::AcaoMenu::AlternarBarra => self.alternar_barra(),
                    dock::AcaoMenu::Minimizar(modo) => self.definir_minimizar(modo),
                    dock::AcaoMenu::Preferencias => self.abrir_preferencias(),
                    dock::AcaoMenu::Posicao(p) => self.definir_posicao_do_dock(p),
                    dock::AcaoMenu::AbrirPasta(p) => {
                        if let Ok(f) = finder::Finder::abrir(&self.gpu, p) {
                            self.finders.push(f);
                        }
                    }
                    dock::AcaoMenu::Nada => {}
                }
                self.lembrar_janelas();
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED if no_dock => {
                self.dock.cancelar_arrasto();
                None
            }
            dock::WM_APPBAR => {
                match wp.0 {
                    ABN_FULLSCREENAPP => {
                        encaixe::TELA_CHEIA.store(lp.0 != 0, Ordering::Relaxed);
                        self.dock.definir_tela_cheia(lp.0 != 0);
                    }
                    ABN_POSCHANGED => {
                        let _ = self.dock.reposicionar(&self.gpu);
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_APP_ARRASTO => {
                let h = HWND(wp.0 as *mut _);
                if lp.0 != 0 {
                    if let Some(a) = self.arrasto.take() {
                        a.terminar(hwnd);
                    }
                    self.arrasto = layouts::Arrasto::comecar(hwnd, h);
                } else {
                    match self.arrasto.take() {
                        Some(a) => a.terminar(hwnd),
                        None => encaixe::ajustar(h),
                    }
                }
                Some(LRESULT(0))
            }
            WM_APP_PASTA => {
                // Janelas novas do Explorador viram janelas do Finder.
                for (h, destino) in pastas::prontos() {
                    // h nulo: pedido do próprio Cais, sem janela do Explorador.
                    let do_explorador = !h.0.is_null();
                    if do_explorador && unsafe { !IsWindow(h).as_bool() } {
                        continue;
                    }
                    let pasta = match destino {
                        pastas::Destino::Pasta(p) => p,
                        pastas::Destino::Computador => std::path::PathBuf::from(arquivos::COMPUTADOR),
                        pastas::Destino::Inicio => finder::pasta_inicial(),
                    };
                    if let Ok(f) = finder::Finder::abrir(&self.gpu, pasta) {
                        self.finders.push(f);
                        if do_explorador {
                            pastas::fechar_explorador(h);
                        }
                    }
                }
                Some(LRESULT(0))
            }
            WM_APP_MINIMIZACAO => {
                let h = HWND(wp.0 as *mut _);
                if lp.0 != 0 {
                    match self.config.minimizar_para.as_str() {
                        "lateral" => {
                            let gpu = &self.gpu;
                            let m = lateral::monitor_da_janela(h);
                            let i = self.laterais.iter().position(|l| l.monitor() == m).unwrap_or(0);
                            if let Some(l) = self.laterais.get_mut(i) {
                                l.minimizou(gpu, h);
                            }
                            if self.usar_genio() {
                                self.comecar_genio(i, h, None);
                            }
                        }
                        "dock" => self.dock.minimizou(h),
                        _ => {}
                    }
                } else {
                    encaixe::ajustar(h);
                    self.dock.restaurou(h);
                    for l in &mut self.laterais {
                        l.restaurou(&self.gpu, h);
                    }
                }
                Some(LRESULT(0))
            }
            WM_APP_BARRA => {
                // A barra do Explorer se mexeu: a nossa bandeja volta a ser
                // a primeira da fila.
                bandeja::ficar_na_frente();
                if self.config.esconder_barra && !self.opcoes.medindo() {
                    barra::esconder_janelas();
                }
                Some(LRESULT(0))
            }
            m if no_dock && m as isize == MSG_SAIR.load(Ordering::Relaxed) => {
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            // Pedido educado para fechar (ex.: taskkill sem /F): sai limpo.
            WM_CLOSE => {
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            WM_ENDSESSION if no_dock => {
                if wp.0 != 0 {
                    self.encerrar();
                }
                Some(LRESULT(0))
            }
            m if no_dock && m as isize == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                // O Explorer reiniciou e recriou a barra (o aviso que nós
                // mesmos mandamos para a bandeja não conta).
                if bandeja::EMITINDO.load(Ordering::Relaxed) {
                    return Some(LRESULT(0));
                }
                if !self.opcoes.medindo() {
                    bandeja::reassumir();
                }
                if self.config.esconder_barra && !self.opcoes.medindo() {
                    barra::esconder();
                }
                self.dock.registrar_de_novo(&self.gpu);
                self.topo.registrar_de_novo(&self.gpu);
                for l in &mut self.laterais {
                    l.registrar_de_novo(&self.gpu);
                }
                for t in &mut self.topos_extra {
                    t.registrar_de_novo(&self.gpu);
                }
                Some(LRESULT(0))
            }
            WM_MBUTTONUP if no_dock => {
                let animando = self.dock.animando();
                self.medidor.alternar(animando);
                Some(LRESULT(0))
            }
            WM_TIMER => match wp.0 {
                TIMER_MEDIDOR => {
                    let animando = self.dock.animando() || self.opcoes.estresse;
                    self.medidor.atualizar(animando);
                    Some(LRESULT(0))
                }
                dock::TIMER_OCULTAR => {
                    self.dock.ocultar_se_ocioso();
                    Some(LRESULT(0))
                }
                encaixe::TIMER_ENCAIXE => {
                    unsafe {
                        let _ = KillTimer(hwnd, encaixe::TIMER_ENCAIXE);
                    }
                    encaixe::ajustar_todas();
                    Some(LRESULT(0))
                }
                layouts::TIMER_LAYOUT => {
                    match &mut self.arrasto {
                        Some(a) => a.olhar(&self.gpu),
                        None => unsafe {
                            let _ = KillTimer(hwnd, layouts::TIMER_LAYOUT);
                        },
                    }
                    Some(LRESULT(0))
                }
                TIMER_JANELAS => {
                    unsafe {
                        let _ = KillTimer(hwnd, TIMER_JANELAS);
                    }
                    RELEITURAS.fetch_add(1, Ordering::Relaxed);
                    if bandeja::limpar_mortos() {
                        self.atualizar_bandeja();
                    }
                    bandeja::ficar_na_frente();
                    self.dock.atualizar_apps(&self.gpu);
                    for l in &mut self.laterais {
                        l.conferir(&self.gpu);
                    }
                    self.lembrar_janelas();
                    Some(LRESULT(0))
                }
                TIMER_MEDICAO => {
                    unsafe {
                        let _ = KillTimer(hwnd, TIMER_MEDICAO);
                    }
                    let texto = self.relatorio();
                    println!("{texto}");
                    if let Some(caminho) = &self.opcoes.saida {
                        let _ = std::fs::write(caminho, &texto);
                    }
                    unsafe { PostQuitMessage(0) };
                    Some(LRESULT(0))
                }
                _ => None,
            },
            WM_DPICHANGED => {
                if no_dock {
                    let _ = self.dock.reposicionar(&self.gpu);
                } else if hwnd == self.medidor.hwnd() {
                    let _ = self.medidor.reposicionar(&self.gpu);
                }
                Some(LRESULT(0))
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE if no_dock => {
                // Claro/escuro ou cor de destaque do Windows mudou.
                if msg == WM_SETTINGCHANGE && tema::sistema_mudou() {
                    self.tema_mudou();
                }
                if !self.opcoes.medindo() {
                    // Monitor ligado, desligado ou com outra resolução.
                    self.atualizar_monitores();
                }
                let _ = self.dock.reposicionar(&self.gpu);
                let _ = self.topo.reposicionar(&self.gpu);
                for t in &mut self.topos_extra {
                    let _ = t.reposicionar(&self.gpu);
                }
                for l in &mut self.laterais {
                    l.reposicionar(&self.gpu);
                }
                let _ = self.medidor.reposicionar(&self.gpu);
                None
            }
            _ => None,
        }
    }
}

/// Traz uma janela minimizada de volta e dá o foco a ela. Janela de
/// administrador ignora o ShowWindow de um programa comum; aí tenta pelo
/// mesmo caminho do Alt+Tab. Falso se ela continuou minimizada.
fn restaurar_janela(h: HWND) -> bool {
    unsafe {
        let _ = ShowWindow(h, SW_RESTORE);
        if IsIconic(h).as_bool() {
            SwitchToThisWindow(h, true);
        }
        let _ = SetForegroundWindow(h);
        !IsIconic(h).as_bool()
    }
}

/// Animação de minimizar do próprio Windows: com o gênio ligado ela sai
/// (senão as duas brigam). Só vale enquanto o Cais roda: não é gravada no
/// registro, e ao sair (ou ao desligar o gênio) volta como estava.
static ANIMACAO_ORIGINAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

fn animacao_do_windows(desligar: bool) {
    let tam = std::mem::size_of::<ANIMATIONINFO>() as u32;
    let mut ai = ANIMATIONINFO { cbSize: tam, iMinAnimate: 0 };
    unsafe {
        if desligar {
            if ANIMACAO_ORIGINAL.load(Ordering::Relaxed) < 0 {
                if SystemParametersInfoW(SPI_GETANIMATION, tam, Some(&mut ai as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_err() {
                    return;
                }
                ANIMACAO_ORIGINAL.store(ai.iMinAnimate, Ordering::Relaxed);
            }
            if ANIMACAO_ORIGINAL.load(Ordering::Relaxed) != 0 {
                ai.iMinAnimate = 0;
                let _ = SystemParametersInfoW(SPI_SETANIMATION, tam, Some(&mut ai as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
            }
        } else {
            let o = ANIMACAO_ORIGINAL.swap(-1, Ordering::Relaxed);
            if o > 0 {
                ai.iMinAnimate = o;
                let _ = SystemParametersInfoW(SPI_SETANIMATION, tam, Some(&mut ai as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
            }
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // Menu do Windows aberto pelo Finder: os submenus dele ("Abrir com",
    // "Enviar para") precisam destas mensagens, mesmo com a App ocupada.
    if let Some(r) = arquivos::mensagem_do_menu(msg, wp, lp) {
        return r;
    }
    let tratado = APP.with(|celula| {
        let mut guarda = celula.try_borrow_mut().ok()?;
        guarda.as_mut()?.mensagem(hwnd, msg, wp, lp)
    });
    match tratado {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// O Windows avisa quando janelas aparecem, somem ou mudam. Juntamos vários
/// avisos seguidos num só (150 ms) antes de reler a lista de apps.
unsafe extern "system" fn evento_de_janela(
    _: HWINEVENTHOOK,
    evento: u32,
    hwnd: HWND,
    id_objeto: i32,
    id_filho: i32,
    _: u32,
    _: u32,
) {
    if id_objeto != 0 || id_filho != 0 || hwnd.0.is_null() {
        return;
    }
    let dock = HWND(DOCK_HWND.load(Ordering::Relaxed) as *mut _);
    if evento == EV_ARRASTO_INICIO || evento == EV_ARRASTO_FIM {
        if evento == EV_ARRASTO_FIM {
            lateral::anotar_monitor(hwnd);
        }
        let ini = (evento == EV_ARRASTO_INICIO) as isize;
        let _ = PostMessageW(dock, WM_APP_ARRASTO, WPARAM(hwnd.0 as usize), LPARAM(ini));
        return;
    }
    if evento == EV_PRIMEIRO_PLANO {
        arredondar::aplicar(hwnd);
        lateral::anotar_monitor(hwnd);
        encaixe::ajustar(hwnd);
        let topo = HWND(TOPO_HWND.load(Ordering::Relaxed) as *mut _);
        let _ = PostMessageW(topo, topo::WM_PRIMEIRO_PLANO, WPARAM(hwnd.0 as usize), LPARAM(0));
        return;
    }
    if evento == EV_MINIMIZOU || evento == EV_RESTAUROU {
        if evento == EV_RESTAUROU {
            arredondar::aplicar(hwnd);
        }
        // Ao minimizar, anota se ainda der (às vezes o aviso chega antes).
        lateral::anotar_monitor(hwnd);
        let min = (evento == EV_MINIMIZOU) as isize;
        let _ = PostMessageW(dock, WM_APP_MINIMIZACAO, WPARAM(hwnd.0 as usize), LPARAM(min));
        return;
    }
    if evento == EV_MOSTRADA {
        arredondar::aplicar(hwnd);
        pastas::janela_apareceu(hwnd, WM_APP_PASTA);
    }
    if evento == EV_DESTRUIDA {
        lateral::esquecer_monitor(hwnd);
    }
    if evento == EV_MOSTRADA && barra::eh_bandeja(hwnd) {
        let _ = PostMessageW(dock, WM_APP_BARRA, WPARAM(0), LPARAM(0));
        return;
    }
    let conhecida = CONHECIDAS.with(|c| c.borrow().contains(&(hwnd.0 as isize)));
    let relevante = match evento {
        EV_DESTRUIDA | EV_ESCONDIDA | EV_OCULTADA => conhecida,
        _ => !conhecida && apps::eh_janela_de_app(hwnd, true),
    };
    if relevante {
        let k = match evento {
            EV_DESTRUIDA | EV_ESCONDIDA => 0,
            EV_NOME => 1,
            EV_OCULTADA | EV_REVELADA => 2,
            _ => 3,
        };
        EVENTOS[k].fetch_add(1, Ordering::Relaxed);
        SetTimer(dock, TIMER_JANELAS, 150, None);
    }
}

fn observar_janelas() {
    let flags = WINEVENT_FORA_DO_PROCESSO | WINEVENT_IGNORAR_PROPRIO;
    unsafe {
        SetWinEventHook(EV_DESTRUIDA, EV_ESCONDIDA, None, Some(evento_de_janela), 0, 0, flags);
        SetWinEventHook(EV_NOME, EV_NOME, None, Some(evento_de_janela), 0, 0, flags);
        SetWinEventHook(
            EV_PRIMEIRO_PLANO,
            EV_PRIMEIRO_PLANO,
            None,
            Some(evento_de_janela),
            0,
            0,
            flags,
        );
        SetWinEventHook(EV_OCULTADA, EV_REVELADA, None, Some(evento_de_janela), 0, 0, flags);
        SetWinEventHook(EV_MINIMIZOU, EV_RESTAUROU, None, Some(evento_de_janela), 0, 0, flags);
        SetWinEventHook(EV_ARRASTO_INICIO, EV_ARRASTO_FIM, None, Some(evento_de_janela), 0, 0, flags);
    }
}

/// Laço principal. Parado: dorme em GetMessage (0% de CPU).
/// Animando: processa mensagens e desenha um quadro por atualização da tela.
fn laco() {
    let mut msg = MSG::default();
    loop {
        let animar = APP.with(|c| c.borrow().as_ref().map_or(false, |a| a.precisa_desenhar()));
        unsafe {
            if animar {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        return;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                APP.with(|c| {
                    if let Some(app) = c.borrow_mut().as_mut() {
                        app.quadro();
                    }
                });
            } else {
                if GetMessageW(&mut msg, None, 0, 0).0 <= 0 {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

fn main() -> Result<()> {
    let inicio = Instant::now();
    // Se o Cais cair no meio de um gênio de volta, a janela não pode ficar
    // invisível (o gancho roda antes do abort).
    let gancho = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        genio::mostrar_todas();
        gancho(info);
    }));
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // OLE (e não só COM): arrastar e soltar arquivos no Finder precisa.
        windows::Win32::System::Ole::OleInitialize(None)?;
    }
    let opcoes = ler_opcoes();
    if opcoes.restaurar_barra {
        barra::mostrar();
        return Ok(());
    }
    if opcoes.testar_lateral {
        janela::registrar(Some(wndproc))?;
        let gpu = gpu::Gpu::new()?;
        let cfg = config::Config::carregar();
        let mut l = lateral::Lateral::new(&gpu, false, cfg.opcoes_lateral(), janela::hmonitor_principal())?;
        l.ler_minimizadas(&gpu);
        let fim = Instant::now() + std::time::Duration::from_secs(5);
        let mut msg = MSG::default();
        while Instant::now() < fim {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            l.passo(1.0 / 60.0);
            if l.sujo {
                let _ = l.desenhar(&gpu);
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        return Ok(());
    }
    if opcoes.testar_finder {
        janela::registrar(Some(wndproc))?;
        let t0 = Instant::now();
        let r = arquivos::imagem_do_shell(&finder::pasta_inicial().join(".aws").to_string_lossy(), 96, windows::Win32::UI::Shell::SIIGBF_ICONONLY);
        println!("icone direto: {:?} em {} ms", r.as_ref().map(|p| (p.0, p.1)), t0.elapsed().as_millis());
        let gpu = gpu::Gpu::new()?;
        let mut f = finder::Finder::abrir_com(&gpu, finder::pasta_inicial(), false)?;
        unsafe {
            let _ = SetWindowPos(f.hwnd(), HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        let fim = Instant::now() + std::time::Duration::from_secs(5);
        let mut msg = MSG::default();
        while Instant::now() < fim {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.hwnd == f.hwnd() {
                        if f.mensagem(&gpu, msg.message, msg.wParam, msg.lParam).is_none() {
                            DefWindowProcW(msg.hwnd, msg.message, msg.wParam, msg.lParam);
                        }
                    } else {
                        DispatchMessageW(&msg);
                    }
                }
            }
            if f.sujo {
                let _ = f.desenhar(&gpu);
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        drop(f);
        return Ok(());
    }
    if opcoes.testar_bandeja {
        return testar_bandeja();
    }
    if opcoes.testar_genio {
        janela::registrar(Some(wndproc))?;
        return testar_genio();
    }
    if opcoes.testar_3d {
        janela::registrar(Some(wndproc))?;
        return teste3d::rodar();
    }
    if opcoes.testar_menus {
        menus_uia::testar();
        return Ok(());
    }
    if opcoes.testar_encaixe {
        encaixe::testar();
        return Ok(());
    }
    if opcoes.instalar {
        instalador::instalar_pela_linha_de_comando();
        return Ok(());
    }
    if opcoes.desinstalar {
        instalador::desinstalar_pela_linha_de_comando();
        return Ok(());
    }
    // Um Cais só: abrir de novo com um já aberto não faz nada.
    let unica = if opcoes.medindo() { None } else { Some(instalador::uma_instancia()) };
    if matches!(unica, Some(None)) {
        return Ok(());
    }
    janela::menus_no_tema();
    janela::registrar(Some(wndproc))?;
    MSG_SAIR.store(unsafe { RegisterWindowMessageW(instalador::MSG_SAIR) } as isize, Ordering::Relaxed);
    let msg = unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) };
    TASKBAR_CREATED.store(msg as isize, Ordering::Relaxed);
    let app = match App::new(opcoes, inicio) {
        Ok(a) => a,
        Err(e) => {
            barra::mostrar(); // nunca deixar a barra sumida se o Cais não abriu
            return Err(e);
        }
    };
    APP.with(|c| *c.borrow_mut() = Some(app));
    observar_janelas();
    laco();
    APP.with(|c| {
        if let Some(app) = c.borrow_mut().take() {
            app.encerrar();
        }
    });
    // Solta o "um Cais só" antes de abrir o próximo.
    drop(unica);
    if let Some(apagar) = DESINSTALAR_AO_SAIR.lock().ok().and_then(|g| *g) {
        instalador::remover(apagar);
        return Ok(());
    }
    if REINICIAR.load(Ordering::Relaxed) {
        // A barra de tarefas e as reservas já foram devolvidas: o Cais novo
        // (a versão que estiver no disco agora) começa do zero.
        let exe = REABRIR_EM.lock().ok().and_then(|g| g.clone()).unwrap_or_else(config::caminho_do_cais);
        let _ = std::process::Command::new(exe).spawn();
    }
    Ok(())
}

/// cais.exe --testar-genio: toca o gênio devagar (ida e volta) numa janela
/// já minimizada, sem mexer nela. Não pega mouse nem teclado.
fn testar_genio() -> Result<()> {
    let gpu = gpu::Gpu::new()?;
    let Some(h) = apps::enumerar_janelas().into_iter().map(|(h, _)| h).find(|h| unsafe { IsIconic(*h).as_bool() }) else {
        println!("genio: nenhuma janela minimizada para testar");
        return Ok(());
    };
    let de = genio::retangulo_da_janela(h);
    println!("genio: janela {:?} em {:?}", h, de);
    let (_, trab) = janela::info_monitor(lateral::monitor_da_janela(h));
    let t = trab.top as f32;
    let para = genio::Ret { esq: trab.left as f32 + 30.0, topo: t + 300.0, dir: trab.left as f32 + 300.0, base: t + 480.0 };
    for volta in [false, true] {
        let t0 = Instant::now();
        let palco = genio::Palco::new(&gpu);
        let t1 = Instant::now();
        let Some(mut g) = de.zip(palco).and_then(|(de, p)| genio::Genio::new(p, h, de, para, volta)) else {
            println!("genio: não foi possível criar");
            return Ok(());
        };
        println!(
            "genio: palco em {} ms, efeito na tela em {} ms",
            (t1 - t0).as_millis(),
            t1.elapsed().as_millis()
        );
        g.devagar(2.0);
        let inicio = Instant::now();
        while !g.passo() {
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        println!("genio: {} terminou em {} ms", if volta { "volta" } else { "ida" }, inicio.elapsed().as_millis());
    }
    Ok(())
}