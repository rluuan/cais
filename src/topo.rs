//! Barra de menu no topo da tela: menu do Cais, app em foco e os menus
//! dele, e à direita bateria, busca, central de controle e relógio.
//! Acorda só quando algo muda: troca de app, clique, mouse em cima, a
//! virada de cada minuto (relógio) e mudanças de energia (bateria).

use crate::apps;
use crate::gpu::Gpu;
use crate::icones;
use crate::sistema::{Estado, Rede};
use crate::janela;
use crate::superficie::{arredondado, cor, retangulo, Superficie};
use crate::vidro::{Vidro, TINTA_TOPO};
use std::collections::HashMap;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::System::Power::*;
use windows::Win32::System::Shutdown::*;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const ALTURA: f32 = 30.0;
const MARGEM: f32 = 10.0;
const FOLGA: f32 = 9.0; // espaço interno de cada item (para o destaque)
const LARG_ICONE: f32 = 32.0;
const LARG_BANDEJA: f32 = 28.0;
/// Ícones da bandeja: 16 DIPs, como na barra de tarefas.
pub const ICONE_BANDEJA: f32 = 16.0;

pub const TIMER_RELOGIO: usize = 5;
/// Espera um instante com o mouse parado num ícone da bandeja.
const TIMER_DICA: usize = 11;
/// O app em primeiro plano mudou (wParam = HWND).
pub const WM_PRIMEIRO_PLANO: u32 = WM_APP + 3;
const WM_MOUSELEAVE: u32 = 0x02A3;

const DIAS: [&str; 7] = ["dom", "seg", "ter", "qua", "qui", "sex", "sáb"];
const DIAS_EN: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MESES_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MESES: [&str; 12] = [
    "jan", "fev", "mar", "abr", "mai", "jun", "jul", "ago", "set", "out", "nov", "dez",
];

#[derive(Clone, Copy, PartialEq)]
enum Alvo {
    Logo,
    App,
    MenuDoApp(i32),
    Janela,
    Som,
    Bateria,
    Rede,
    Busca,
    Controle,
    Relogio,
    /// Ícone do próprio Cais (clique abre as Preferências).
    Cais,
    /// Ícone k da bandeja (Docker, OpenVPN...).
    Bandeja(usize),
    /// Ícones da bandeja que não couberam ao lado dos menus do app.
    Mais,
    /// CPU e memória (opção nas Preferências).
    Monitor,
    /// Ponto da área de trabalho virtual k.
    Area(usize),
}

/// Pedidos que a barra faz à App (Preferências, reiniciar, sair).
#[derive(Clone, Copy, PartialEq)]
pub enum PedidoCais {
    Preferencias,
    Monitor,
    Reiniciar,
    Sair,
}

impl Alvo {
    /// Itens que abrem a Central de Controle.
    fn da_central(self) -> bool {
        matches!(self, Alvo::Som | Alvo::Rede | Alvo::Controle)
    }
}

pub struct Topo {
    sup: Superficie,
    vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    texto: IDWriteTextFormat,
    negrito: IDWriteTextFormat,
    /// Texto menor (CPU/memória).
    pequeno: IDWriteTextFormat,
    larg_dip: f32,
    reservar: bool,
    ultima_reserva: Option<RECT>,
    /// Janela do app em foco (fora do Cais) e o que sabemos dela.
    foco: HWND,
    exe_foco: String,
    nome_foco: String,
    menus_foco: Vec<String>,
    /// Os menus vieram da Automação de Interface (app moderno), não do
    /// menu clássico do Windows.
    menus_modernos: bool,
    nomes: HashMap<String, String>,
    bateria: Option<(u8, bool)>,
    alvos: Vec<(f32, f32, Alvo)>,
    hover: Option<Alvo>,
    aberto: Option<Alvo>,
    rastreando: bool,
    /// Monitor desta barra (há uma barra em cada monitor).
    monitor: HMONITOR,
    /// Wi-Fi/rede e som, vindos da thread do sistema.
    rede: Rede,
    wifi: Option<bool>,
    volume: f32,
    mudo: bool,
    tem_som: bool,
    /// Clicaram num item que abre/fecha a Central de Controle.
    pub pedido_central: bool,
    /// Clicaram na lupa (a App abre a busca do Cais).
    pub pedido_busca: bool,
    /// Clicaram no relógio: a App abre o calendário.
    pub pedido_calendario: bool,
    /// Escolheram "Preferências do Cais" no menu do logo.
    pub pedido_cais: std::cell::Cell<Option<PedidoCais>>,
    /// Clique num ícone da bandeja: (índice, direito, x, y na tela).
    pub pedido_bandeja: Option<(usize, bool, i32, i32)>,
    /// Clicaram no ponto de uma área de trabalho virtual.
    pub pedido_area: Option<usize>,
    /// Área virtual atual e quantas existem (pontinhos só com 2 ou mais).
    areas: (usize, usize),
    /// Texto de CPU/memória na barra (None = opção desligada).
    monitor_texto: Option<String>,
    bandeja: Vec<Option<ID2D1Bitmap1>>,
    /// Nome de cada ícone da bandeja (aparece ao parar o mouse em cima).
    dicas: Vec<String>,
    /// Ícones da bandeja que não couberam (ficam no botão "mais").
    ocultos: Vec<usize>,
    balao: Option<crate::dica::Balao>,
    pub central_aberta: bool,
    pub sujo: bool,
}

impl Drop for Topo {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.sup.hwnd);
        }
    }
}

impl Topo {
    pub fn new(gpu: &Gpu, reservar: bool, monitor: HMONITOR) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE)?;
        janela::preparar_vidro(hwnd);
        if reservar {
            janela::registrar_appbar(hwnd, crate::dock::WM_APPBAR);
        }
        let mut ultima_reserva = None;
        posicionar_janela(hwnd, reservar, monitor, &mut ultima_reserva);
        let sup = Superficie::new(gpu, hwnd)?;
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, TINTA_TOPO)?;
        let escala = sup.dpi / 96.0;
        let (l, a) = janela::tamanho_cliente(hwnd);
        let _ = vidro.definir(0.0, 0.0, l as f32, a as f32, 0.0);
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let fonte = |peso| {
                gpu.dwrite.CreateTextFormat(
                    w!("Segoe UI Variable Text"),
                    None,
                    peso,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    13.5,
                    crate::idioma::localidade(),
                )
            };
            let texto = fonte(DWRITE_FONT_WEIGHT_NORMAL)?;
            let negrito = fonte(DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let pequeno = gpu.dwrite.CreateTextFormat(
                w!("Segoe UI Variable Small"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                12.0,
                crate::idioma::localidade(),
            )?;
            pequeno.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            pequeno.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            for f in [&texto, &negrito] {
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            }
            let mut t = Self {
                sup,
                vidro,
                pincel,
                texto,
                negrito,
                pequeno,
                larg_dip: l as f32 / escala,
                reservar,
                ultima_reserva,
                foco: HWND::default(),
                exe_foco: String::new(),
                nome_foco: String::new(),
                menus_foco: Vec::new(),
                menus_modernos: false,
                nomes: HashMap::new(),
                bateria: None,
                alvos: Vec::new(),
                hover: None,
                aberto: None,
                rastreando: false,
                monitor,
                rede: Rede::Nenhuma,
                wifi: None,
                volume: 0.0,
                mudo: false,
                tem_som: false,
                pedido_central: false,
                pedido_busca: false,
                pedido_calendario: false,
                pedido_cais: std::cell::Cell::new(None),
                pedido_bandeja: None,
                pedido_area: None,
                areas: (0, 1),
                monitor_texto: None,
                bandeja: Vec::new(),
                dicas: Vec::new(),
                ocultos: Vec::new(),
                balao: None,
                central_aberta: false,
                sujo: true,
            };
            t.ler_bateria();
            t.primeiro_plano_mudou(GetForegroundWindow());
            t.agendar_relogio();
            Ok(t)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }


    fn escala(&self) -> f32 {
        self.sup.dpi / 96.0
    }

    pub fn mostrar(&self) {
        unsafe {
            let _ = ShowWindow(self.sup.hwnd, SW_SHOWNOACTIVATE);
        }
    }

    pub fn reposicionar(&mut self, gpu: &Gpu) -> Result<()> {
        posicionar_janela(self.sup.hwnd, self.reservar, self.monitor, &mut self.ultima_reserva);
        self.sup.redimensionar(gpu)?;
        let (l, a) = janela::tamanho_cliente(self.sup.hwnd);
        self.larg_dip = l as f32 / self.escala();
        let _ = self.vidro.definir(0.0, 0.0, l as f32, a as f32, 0.0);
        self.sujo = true;
        Ok(())
    }

    pub fn registrar_de_novo(&mut self, gpu: &Gpu) {
        if self.reservar {
            janela::registrar_appbar(self.sup.hwnd, crate::dock::WM_APPBAR);
            self.ultima_reserva = None;
        }
        let _ = self.reposicionar(gpu);
    }

    pub fn sair_do_windows(&self) {
        if self.reservar {
            janela::remover_appbar(self.sup.hwnd);
        }
    }

    /// O relógio acorda na virada exata de cada minuto, e só nela.
    fn agendar_relogio(&self) {
        unsafe {
            let t = GetLocalTime();
            let ms = (60 - t.wSecond as u32) * 1000 - t.wMilliseconds as u32 + 30;
            SetTimer(self.sup.hwnd, TIMER_RELOGIO, ms, None);
        }
    }

    fn ler_bateria(&mut self) {
        let mut s = SYSTEM_POWER_STATUS::default();
        self.bateria = unsafe { GetSystemPowerStatus(&mut s) }.ok().and_then(|_| {
            // 128 = sem bateria (desktop), 255 = desconhecido
            if s.BatteryFlag & 128 != 0 || s.BatteryFlag == 255 || s.BatteryLifePercent > 100 {
                None
            } else {
                Some((s.BatteryLifePercent, s.ACLineStatus == 1))
            }
        });
    }

    /// Novo estado de rede/som: redesenha só se o que aparece na barra mudou.
    /// Área de trabalho virtual atual e quantas existem.
    pub fn definir_areas(&mut self, atual: usize, total: usize) {
        if self.areas != (atual, total) {
            self.areas = (atual, total);
            self.sujo = true;
        }
    }

    pub fn definir_sistema(&mut self, e: &Estado) {
        let volume = (e.volume * 3.0).ceil(); // a barra só mostra 0 a 3 ondas
        let mudou = self.rede != e.rede
            || self.wifi != e.wifi
            || (self.volume * 3.0).ceil() != volume
            || self.mudo != e.mudo
            || self.tem_som != e.tem_som;
        self.rede = e.rede.clone();
        self.wifi = e.wifi;
        self.volume = e.volume;
        self.mudo = e.mudo;
        self.tem_som = e.tem_som;
        if mudou {
            self.sujo = true;
        }
    }

    /// O clique em (lParam) vai cair num item da Central de Controle?
    pub fn clique_na_central(&self, lp: LPARAM) -> bool {
        let x = (lp.0 & 0xffff) as i16 as f32 / self.escala();
        self.acertar(x).map_or(false, |a| a.da_central())
    }

    /// Canto inferior direito da barra (onde a Central se encosta).
    pub fn canto(&self) -> (i32, i32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.sup.hwnd, &mut rc);
        }
        (rc.right, rc.bottom)
    }

    /// Outro app veio para frente: nome dele e, se for um programa clássico
    /// com barra de menus, os títulos dos menus.
    pub fn primeiro_plano_mudou(&mut self, h: HWND) {
        if h.0.is_null() {
            return;
        }
        let pid = apps::pid_do_app(h);
        if pid == std::process::id() {
            return; // o próprio Cais (menu aberto etc.)
        }
        let exe = apps::exe_do_pid(pid).unwrap_or_default().to_lowercase();
        if !self.nomes.contains_key(&exe) {
            let nome = if exe.is_empty() {
                "Windows".to_string()
            } else {
                apps::nome_do_exe(&exe)
            };
            self.nomes.insert(exe.clone(), nome);
        }
        self.nome_foco = apps::nome_de_pwa(h, &exe)
            .or_else(|| nome_pelo_titulo(&exe, h))
            .unwrap_or_else(|| self.nomes[&exe].clone());
        // Mesma chave do dock: app instalado pelo navegador é um app à parte.
        self.exe_foco = apps::chave_da_janela(h, exe);
        self.foco = h;
        self.menus_foco = titulos_de_menu(h);
        self.menus_modernos = false;
        if self.menus_foco.is_empty() {
            // Sem menu clássico: talvez o app desenhe o próprio (a resposta
            // chega depois, pela App).
            crate::menus_uia::pedir(h);
        }
        self.sujo = true;
    }

    /// Chegaram os menus de um app moderno.
    pub fn menus_modernos_chegaram(&mut self, h: HWND, titulos: &[String]) {
        if h == self.foco && self.menus_foco.is_empty() && !titulos.is_empty() {
            self.menus_foco = titulos.to_vec();
            self.menus_modernos = true;
            self.sujo = true;
        }
    }

    pub fn mensagem(&mut self, gpu: &Gpu, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let e = self.escala();
        let x = (lp.0 & 0xffff) as i16 as f32 / e;
        match msg {
            WM_MOUSEMOVE => {
                if !self.rastreando {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.sup.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                    self.rastreando = true;
                }
                let novo = self.acertar(x);
                if novo != self.hover {
                    self.hover = novo;
                    self.sujo = true;
                    // Parou num ícone da bandeja: o nome aparece num instante.
                    self.esconder_dica();
                    if matches!(novo, Some(Alvo::Bandeja(_))) {
                        unsafe {
                            SetTimer(self.sup.hwnd, TIMER_DICA, 500, None);
                        }
                    }
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_DICA => {
                unsafe {
                    let _ = KillTimer(self.sup.hwnd, TIMER_DICA);
                }
                if let Some(Alvo::Bandeja(k)) = self.hover {
                    self.mostrar_dica(k);
                }
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                self.esconder_dica();
                self.rastreando = false;
                if self.hover.take().is_some() {
                    self.sujo = true;
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                match self.acertar(x) {
                    Some(Alvo::Cais) => self.menu_cais(gpu, x),
                    Some(Alvo::Bandeja(k)) => self.pedir_bandeja(k, true),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                self.esconder_dica();
                if let Some(i) = self.alvos.iter().position(|a| x >= a.0 && x <= a.1) {
                    self.acionar(gpu, i);
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_RELOGIO => {
                self.agendar_relogio();
                self.sujo = true;
                Some(LRESULT(0))
            }
            WM_TIMECHANGE => {
                self.agendar_relogio();
                self.sujo = true;
                Some(LRESULT(0))
            }
            WM_POWERBROADCAST => {
                self.ler_bateria();
                self.sujo = true;
                Some(LRESULT(1))
            }
            WM_PRIMEIRO_PLANO => {
                self.primeiro_plano_mudou(HWND(wp.0 as *mut _));
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn acertar(&self, x: f32) -> Option<Alvo> {
        self.alvos
            .iter()
            .find(|a| x >= a.0 && x <= a.1)
            .map(|a| a.2)
    }

    /// Abre o menu (ou executa a ação) do item clicado.
    fn acionar(&mut self, gpu: &Gpu, i: usize) {
        let (x0, _, alvo) = self.alvos[i];
        // O relógio ainda abre as notificações do Windows.
        match alvo {
            a if a.da_central() => {
                self.pedido_central = true;
                return;
            }
            Alvo::Busca => {
                self.pedido_busca = true;
                return;
            }
            Alvo::Cais => {
                self.pedido_cais.set(Some(PedidoCais::Preferencias));
                return;
            }
            Alvo::Bandeja(k) => {
                self.pedir_bandeja(k, false);
                return;
            }
            Alvo::Mais => {
                self.menu_mais(gpu, x0);
                return;
            }
            Alvo::Monitor => {
                self.pedido_cais.set(Some(PedidoCais::Monitor));
                return;
            }
            Alvo::Relogio => {
                self.pedido_calendario = true;
                return;
            }
            Alvo::Bateria => return apps::abrir("ms-settings:batterysaver"),
            Alvo::Area(k) => {
                self.pedido_area = Some(k);
                return;
            }
            _ => {}
        }
        self.aberto = Some(alvo);
        self.sujo = true;
        let _ = self.desenhar(gpu);

        let e = self.escala();
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.sup.hwnd, &mut rc);
        }
        let px = rc.left + (x0 * e) as i32;
        let py = rc.bottom;
        let foco = self.foco;
        match alvo {
            Alvo::Logo => self.menu_logo(px, py),
            Alvo::App => self.menu_app(px, py),
            Alvo::Janela => self.menu_janela(px, py),
            Alvo::MenuDoApp(k) if self.menus_modernos => crate::menus_uia::abrir(foco, k as usize),
            Alvo::MenuDoApp(k) => menu_classico(self.sup.hwnd, foco, k, px, py),
            _ => {}
        }
        self.aberto = None;
        self.hover = None;
        self.sujo = true;
    }

    fn popup(&self, itens: &[(u32, String, bool)], x: i32, y: i32) -> u32 {
        unsafe {
            let Ok(m) = CreatePopupMenu() else { return 0 };
            for (id, texto, ativo) in itens {
                if *id == 0 {
                    let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
                } else {
                    let w: Vec<u16> = texto.encode_utf16().chain(Some(0)).collect();
                    let flags = if *ativo { MF_STRING } else { MF_STRING | MF_GRAYED };
                    let _ = AppendMenuW(m, flags, *id as usize, PCWSTR(w.as_ptr()));
                }
            }
            let _ = SetForegroundWindow(self.sup.hwnd);
            let cmd = TrackPopupMenu(
                m,
                TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN,
                x,
                y,
                0,
                self.sup.hwnd,
                None,
            )
            .0 as u32;
            let _ = DestroyMenu(m);
            cmd
        }
    }

    pub fn definir_monitor(&mut self, texto: Option<String>) {
        if texto != self.monitor_texto {
            self.monitor_texto = texto;
            self.sujo = true;
        }
    }

    /// Ícones da bandeja que esta barra mostra (bitmap de cada um).
    pub fn definir_bandeja(&mut self, v: Vec<Option<ID2D1Bitmap1>>, dicas: Vec<String>) {
        self.bandeja = v;
        self.dicas = dicas;
        self.sujo = true;
    }

    fn esconder_dica(&mut self) {
        unsafe {
            let _ = KillTimer(self.sup.hwnd, TIMER_DICA);
        }
        if let Some(b) = &self.balao {
            b.esconder();
        }
    }

    /// Nome do ícone k da bandeja, logo abaixo dele.
    fn mostrar_dica(&mut self, k: usize) {
        let Some(texto) = self.dicas.get(k).filter(|t| !t.trim().is_empty()).cloned() else { return };
        let Some(&(x0, x1, _)) = self.alvos.iter().find(|a| a.2 == Alvo::Bandeja(k)) else { return };
        if self.balao.is_none() {
            self.balao = crate::dica::Balao::new(self.sup.hwnd);
        }
        let e = self.escala();
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.sup.hwnd, &mut rc);
        }
        let x = rc.left + ((x0 + x1) / 2.0 * e) as i32;
        if let Some(b) = &self.balao {
            b.mostrar(&texto, x, rc.bottom + 4);
        }
    }

    /// Clique num ícone da bandeja: a App manda a mensagem ao programa,
    /// com o ponto (na tela) logo abaixo do ícone.
    fn pedir_bandeja(&mut self, k: usize, direito: bool) {
        let Some(&(x0, x1, _)) = self.alvos.iter().find(|a| a.2 == Alvo::Bandeja(k)) else { return };
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.sup.hwnd, &mut rc);
        }
        let x = rc.left + ((x0 + x1) / 2.0 * self.escala()) as i32;
        self.pedido_bandeja = Some((k, direito, x, rc.bottom));
    }

    /// Botão "mais": lista os ícones da bandeja que não couberam. Escolher um
    /// é o mesmo que clicar nele (abre o app ou o menu dele).
    fn menu_mais(&mut self, gpu: &Gpu, x0: f32) {
        self.aberto = Some(Alvo::Mais);
        self.sujo = true;
        let _ = self.desenhar(gpu);
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.sup.hwnd, &mut rc);
        }
        let px = rc.left + (x0 * self.escala()) as i32;
        let itens: Vec<(u32, String, bool)> = self
            .ocultos
            .iter()
            .map(|&k| {
                let nome = self.dicas.get(k).map(|d| d.lines().next().unwrap_or("").trim().to_string()).filter(|d| !d.is_empty());
                (k as u32 + 1, nome.unwrap_or_else(|| format!("{} {}", crate::idioma::t("Ícone", "Icon"), k + 1)), true)
            })
            .collect();
        let escolha = self.popup(&itens, px, rc.bottom);
        if escolha > 0 {
            self.pedido_bandeja = Some((escolha as usize - 1, false, px, rc.bottom));
        }
        self.aberto = None;
        self.hover = None;
        self.sujo = true;
    }

    /// Clique direito no ícone do Cais: Preferências, Reiniciar, Sair.
    fn menu_cais(&mut self, gpu: &Gpu, x: f32) {
        let Some(&(x0, _, _)) = self.alvos.iter().find(|a| a.2 == Alvo::Cais) else { return };
        let _ = x;
        self.aberto = Some(Alvo::Cais);
        self.sujo = true;
        let _ = self.desenhar(gpu);
        let mut rc = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.sup.hwnd, &mut rc);
        }
        let px = rc.left + (x0 * self.escala()) as i32;
        let s = |t: &str| t.to_string();
        let itens = [
            (1, s(crate::idioma::t("Preferências do Cais…", "Cais Settings…")), true),
            (4, s(crate::idioma::t("Monitor do sistema…", "System monitor…")), true),
            (0, s(""), true),
            (2, s(crate::idioma::t("Reiniciar o Cais", "Restart Cais")), true),
            (3, s(crate::idioma::t("Sair do Cais", "Quit Cais")), true),
        ];
        let pedido = match self.popup(&itens, px, rc.bottom) {
            1 => Some(PedidoCais::Preferencias),
            2 => Some(PedidoCais::Reiniciar),
            3 => Some(PedidoCais::Sair),
            4 => Some(PedidoCais::Monitor),
            _ => None,
        };
        self.pedido_cais.set(pedido);
        self.aberto = None;
        self.hover = None;
        self.sujo = true;
    }

    fn menu_logo(&self, x: i32, y: i32) {
        let s = |t: &str| t.to_string();
        let itens = [
            (1, s(crate::idioma::t("Sobre o Cais", "About Cais")), true),
            (9, s(crate::idioma::t("Preferências do Cais…", "Cais Settings…")), true),
            (10, s(crate::idioma::t("Reiniciar o Cais", "Restart Cais")), true),
            (11, s(crate::idioma::t("Sair do Cais", "Quit Cais")), true),
            (0, s(""), true),
            (2, s(crate::idioma::t("Configurações do Windows…", "Windows Settings…")), true),
            (3, s(crate::idioma::t("Gerenciador de Tarefas", "Task Manager")), true),
            (0, s(""), true),
            (4, s(crate::idioma::t("Suspender", "Sleep")), true),
            (5, s(crate::idioma::t("Reiniciar…", "Restart…")), true),
            (6, s(crate::idioma::t("Desligar…", "Shut Down…")), true),
            (0, s(""), true),
            (7, s(crate::idioma::t("Bloquear tela", "Lock Screen")), true),
            (8, s(crate::idioma::t("Encerrar sessão…", "Log Out…")), true),
        ];
        match self.popup(&itens, x, y) {
            1 => self.sobre(),
            9 => self.pedido_cais.set(Some(PedidoCais::Preferencias)),
            10 => self.pedido_cais.set(Some(PedidoCais::Reiniciar)),
            11 => self.pedido_cais.set(Some(PedidoCais::Sair)),
            2 => apps::abrir("ms-settings:"),
            3 => apps::abrir("taskmgr.exe"),
            4 => unsafe {
                let _ = SetSuspendState(false, false, false);
            },
            5 if self.confirmar(crate::idioma::t("Reiniciar o computador agora?", "Restart the computer now?")) => {
                apps::executar("shutdown.exe", "/r /t 0")
            }
            6 if self.confirmar(crate::idioma::t("Desligar o computador agora?", "Shut down the computer now?")) => {
                apps::executar("shutdown.exe", "/s /t 0")
            }
            7 => unsafe {
                let _ = LockWorkStation();
            },
            8 if self.confirmar(crate::idioma::t("Encerrar a sessão agora? Apps abertos serão fechados.", "Log out now? Open apps will be closed.")) => unsafe {
                let _ = ExitWindowsEx(EWX_LOGOFF, SHTDN_REASON_MAJOR_OTHER);
            },
            _ => {}
        }
    }

    fn menu_app(&self, x: i32, y: i32) {
        let n = &self.nome_foco;
        let itens = [
            (1, format!("{} {n}", crate::idioma::t("Ocultar", "Hide")), true),
            (2, crate::idioma::t("Ocultar outros", "Hide Others").to_string(), true),
            (3, crate::idioma::t("Mostrar tudo", "Show All").to_string(), true),
            (0, String::new(), true),
            (4, format!("{} {n}", crate::idioma::t("Encerrar", "Quit")), true),
        ];
        let cmd = self.popup(&itens, x, y);
        let janelas = apps::enumerar_janelas();
        let do_app = |exe: &String| *exe == self.exe_foco;
        unsafe {
            for (h, exe) in &janelas {
                match cmd {
                    1 if do_app(exe) => {
                        let _ = ShowWindow(*h, SW_MINIMIZE);
                    }
                    2 if !do_app(exe) => {
                        let _ = ShowWindow(*h, SW_MINIMIZE);
                    }
                    3 if IsIconic(*h).as_bool() => {
                        let _ = ShowWindow(*h, SW_SHOWNOACTIVATE);
                    }
                    4 if do_app(exe) => {
                        let _ = PostMessageW(*h, WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                    _ => {}
                }
            }
            if cmd == 2 || cmd == 3 {
                let _ = SetForegroundWindow(self.foco);
            }
        }
    }

    fn menu_janela(&self, x: i32, y: i32) {
        let h = self.foco;
        let maximizada = unsafe { IsZoomed(h).as_bool() };
        let mut itens = vec![
            (1, crate::idioma::t("Minimizar", "Minimize").to_string(), true),
            (
                2,
                if maximizada { crate::idioma::t("Restaurar tamanho", "Restore size") } else { "Zoom" }.to_string(),
                true,
            ),
            (0, String::new(), true),
            (3, crate::idioma::t("Trazer tudo para a frente", "Bring All to Front").to_string(), true),
        ];
        if crate::layouts::ligado() {
            itens.push((5, crate::idioma::t("Organizar janelas neste layout", "Arrange windows in this layout").to_string(), true));
        }
        itens.push((0, String::new(), true));
        itens.push((4, crate::idioma::t("Fechar janela", "Close Window").to_string(), true));
        let cmd = self.popup(&itens, x, y);
        unsafe {
            match cmd {
                1 => {
                    let _ = ShowWindow(h, SW_MINIMIZE);
                }
                2 => {
                    let _ = ShowWindow(h, if maximizada { SW_RESTORE } else { SW_MAXIMIZE });
                    let _ = SetForegroundWindow(h);
                }
                3 => {
                    let janelas = apps::enumerar_janelas();
                    for (j, exe) in janelas.iter().rev() {
                        if *exe == self.exe_foco {
                            if IsIconic(*j).as_bool() {
                                let _ = ShowWindow(*j, SW_RESTORE);
                            }
                            let _ = SetForegroundWindow(*j);
                        }
                    }
                }
                4 => {
                    let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                5 => crate::layouts::organizar(self.monitor),
                _ => {
                    let _ = SetForegroundWindow(h);
                }
            }
        }
    }

    fn confirmar(&self, texto: &str) -> bool {
        let w: Vec<u16> = texto.encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(w.as_ptr()),
                w!("Cais"),
                MB_YESNO | MB_ICONQUESTION | MB_TOPMOST,
            ) == IDYES
        }
    }

    fn sobre(&self) {
        let texto = format!(
            "Cais {}\n\n{}",
            env!("CARGO_PKG_VERSION"),
            crate::idioma::t(
                "Dock, barra de menu, Palco e Arquivos para Windows.\nOpen source, leve e 100% local: o Cais não tem nenhum código de rede.",
                "Dock, menu bar, Stage and Files for Windows.\nOpen source, light and 100% local: Cais has no network code at all."
            )
        );
        let w: Vec<u16> = texto.encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(w.as_ptr()),
                PCWSTR(crate::idioma::largo(crate::idioma::t("Sobre o Cais", "About Cais")).as_ptr()),
                MB_OK | MB_ICONINFORMATION | MB_TOPMOST,
            );
        }
    }

    fn relogio(&self) -> String {
        let t = unsafe { GetLocalTime() };
        let (dia, mes) = ((t.wDayOfWeek % 7) as usize, ((t.wMonth.max(1) - 1) % 12) as usize);
        if crate::idioma::ingles() {
            format!("{} {} {}  {:02}:{:02}", DIAS_EN[dia], MESES_EN[mes], t.wDay, t.wHour, t.wMinute)
        } else {
            format!("{} {} de {}  {:02}:{:02}", DIAS[dia], t.wDay, MESES[mes], t.wHour, t.wMinute)
        }
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        // Monta os itens (textos e posições) da esquerda e da direita.
        let layout = |fmt: &IDWriteTextFormat, s: &str| -> Result<(IDWriteTextLayout, f32)> {
            let w: Vec<u16> = s.encode_utf16().collect();
            unsafe {
                let l = gpu.dwrite.CreateTextLayout(&w, fmt, 2000.0, ALTURA)?;
                let mut m = DWRITE_TEXT_METRICS::default();
                l.GetMetrics(&mut m)?;
                Ok((l, m.widthIncludingTrailingWhitespace))
            }
        };
        let mut textos: Vec<(IDWriteTextLayout, f32)> = Vec::new(); // (layout, x)
        let mut alvos: Vec<(f32, f32, Alvo)> = Vec::new();

        let mut x = MARGEM;
        alvos.push((x, x + LARG_ICONE, Alvo::Logo));
        x += LARG_ICONE + 2.0;
        let nome = if self.nome_foco.is_empty() { "Cais" } else { &self.nome_foco };
        let (l, w) = layout(&self.negrito, nome)?;
        textos.push((l, x + FOLGA));
        alvos.push((x, x + w + FOLGA * 2.0, Alvo::App));
        x += w + FOLGA * 2.0;
        for (k, t) in self.menus_foco.iter().enumerate() {
            let (l, w) = layout(&self.texto, t)?;
            textos.push((l, x + FOLGA));
            alvos.push((x, x + w + FOLGA * 2.0, Alvo::MenuDoApp(k as i32)));
            x += w + FOLGA * 2.0;
        }
        let (l, w) = layout(&self.texto, crate::idioma::t("Janela", "Window"))?;
        textos.push((l, x + FOLGA));
        alvos.push((x, x + w + FOLGA * 2.0, Alvo::Janela));

        let mut d = self.larg_dip - MARGEM;
        let (l, w) = layout(&self.texto, &self.relogio())?;
        d -= w + FOLGA * 2.0;
        textos.push((l, d + FOLGA));
        alvos.push((d, d + w + FOLGA * 2.0, Alvo::Relogio));
        d -= LARG_ICONE;
        alvos.push((d, d + LARG_ICONE, Alvo::Controle));
        d -= LARG_ICONE;
        alvos.push((d, d + LARG_ICONE, Alvo::Busca));
        d -= LARG_ICONE;
        alvos.push((d, d + LARG_ICONE, Alvo::Rede));
        if let Some((pct, _)) = self.bateria {
            let (l, w) = layout(&self.texto, &format!("{pct}%"))?;
            let larg = w + 36.0 + FOLGA * 2.0;
            d -= larg;
            textos.push((l, d + FOLGA));
            alvos.push((d, d + larg, Alvo::Bateria));
        }
        if self.tem_som {
            d -= LARG_ICONE;
            alvos.push((d, d + LARG_ICONE, Alvo::Som));
        }
        d -= LARG_ICONE;
        alvos.push((d, d + LARG_ICONE, Alvo::Cais));
        if let Some(m) = &self.monitor_texto {
            let (l, w) = layout(&self.pequeno, m)?;
            d -= w + FOLGA * 2.0;
            textos.push((l, d + FOLGA));
            alvos.push((d, d + w + FOLGA * 2.0, Alvo::Monitor));
        }
        // Pontinhos das áreas de trabalho virtuais (só com duas ou mais).
        let (area_atual, areas_total) = self.areas;
        if areas_total > 1 {
            d -= 6.0;
            for k in (0..areas_total.min(9)).rev() {
                d -= LARG_AREA;
                alvos.push((d, d + LARG_AREA, Alvo::Area(k)));
            }
            d -= 6.0;
        }
        // Ícones da bandeja (Docker, OpenVPN...), da direita para a esquerda,
        // sem passar por cima dos menus do app em foco.
        let limite = alvos
            .iter()
            .filter(|a| matches!(a.2, Alvo::Logo | Alvo::App | Alvo::MenuDoApp(_) | Alvo::Janela))
            .map(|a| a.1)
            .fold(0.0f32, f32::max)
            + 16.0;
        // Os que não cabem vão para o botão "mais" (antes sumiam sem aviso).
        let n = self.bandeja.len();
        let cabem = ((d - limite) / LARG_BANDEJA).floor().max(0.0) as usize;
        let mostrar = if cabem >= n { n } else { cabem.saturating_sub(1) };
        self.ocultos = (0..n - mostrar).collect();
        for k in (n - mostrar..n).rev() {
            d -= LARG_BANDEJA;
            alvos.push((d, d + LARG_BANDEJA, Alvo::Bandeja(k)));
        }
        if !self.ocultos.is_empty() && cabem > 0 {
            d -= LARG_BANDEJA;
            alvos.push((d, d + LARG_BANDEJA, Alvo::Mais));
        }
        let bandeja = &self.bandeja;

        let destaque = self
            .aberto
            .or(self.central_aberta.then_some(Alvo::Controle))
            .or(self.hover);
        let (rede, wifi, volume, mudo) = (&self.rede, self.wifi, self.volume, self.mudo);
        let aberto = self.aberto;
        let bateria = self.bateria;
        let pincel = &self.pincel;
        let alvos_ref = &alvos;
        self.sup.desenhar(gpu, |dc| unsafe {
            // Destaque do item sob o mouse (mais forte se o menu está aberto).
            if let Some(a) = destaque {
                if let Some(&(x0, x1, _)) = alvos_ref.iter().find(|t| t.2 == a) {
                    let forte = aberto.is_some();
                    pincel.SetColor(&crate::tema::tinta(if forte { 0.22 } else { 0.12 }));
                    dc.FillRoundedRectangle(
                        &arredondado(retangulo(x0, 3.0, x1, ALTURA - 3.0), 6.0),
                        pincel,
                    );
                }
            }
            pincel.SetColor(&crate::tema::tinta(0.94));
            for (l, x) in &textos {
                dc.DrawTextLayout(
                    D2D_POINT_2F { x: *x, y: 0.0 },
                    l,
                    pincel,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            let meio = ALTURA / 2.0;
            for &(x0, x1, a) in alvos_ref {
                let cx = (x0 + x1) / 2.0;
                match a {
                    Alvo::Logo | Alvo::Cais => icone_logo(dc, pincel, cx, meio),
                    Alvo::Mais => {
                        // Três pontinhos.
                        pincel.SetColor(&crate::tema::tinta(0.85));
                        for dx in [-5.0f32, 0.0, 5.0] {
                            dc.FillEllipse(
                                &D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx + dx, y: meio }, radiusX: 1.6, radiusY: 1.6 },
                                pincel,
                            );
                        }
                    }
                    Alvo::Bandeja(k) => {
                        if let Some(Some(b)) = bandeja.get(k) {
                            let t = ICONE_BANDEJA / 2.0;
                            dc.DrawBitmap(
                                b,
                                Some(&retangulo(cx - t, meio - t, cx + t, meio + t)),
                                1.0,
                                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                                None,
                                None,
                            );
                        }
                    }
                    Alvo::Busca => icone_busca(dc, pincel, cx, meio),
                    Alvo::Controle => icone_controle(dc, pincel, cx, meio),
                    Alvo::Som => {
                        icones::som(dc, pincel, cx + 1.0, meio, 1.0, volume, mudo, crate::tema::tinta(0.94))
                    }
                    Alvo::Rede => match rede {
                        Rede::Cabo { .. } => icones::cabo(dc, pincel, cx, meio, 0.95, crate::tema::tinta(0.94)),
                        Rede::Wifi { barras, .. } => {
                            let acesas = icones::faixas_do_sinal(*barras);
                            icones::wifi(dc, pincel, cx, meio - 1.0, 1.0, acesas, (1.0, 1.0, 1.0))
                        }
                        // Sem rede: leque apagado (Wi-Fi) ou caixinhas apagadas.
                        Rede::Nenhuma if wifi.is_some() => {
                            icones::wifi(dc, pincel, cx, meio - 1.0, 1.0, 0, (1.0, 1.0, 1.0))
                        }
                        Rede::Nenhuma => icones::cabo(dc, pincel, cx, meio, 0.95, crate::tema::tinta(0.4)),
                    },
                    Alvo::Bateria => {
                        if let Some((pct, carregando)) = bateria {
                            icone_bateria(dc, pincel, x1 - FOLGA - 26.0, meio, pct, carregando);
                        }
                    }
                    Alvo::Area(k) => {
                        let ponto = D2D1_ELLIPSE { point: D2D_POINT_2F { x: cx, y: meio }, radiusX: 3.5, radiusY: 3.5 };
                        if k == area_atual {
                            pincel.SetColor(&crate::tema::tinta(0.94));
                            dc.FillEllipse(&ponto, pincel);
                        } else {
                            pincel.SetColor(&crate::tema::tinta(0.55));
                            dc.DrawEllipse(&ponto, pincel, 1.2, None);
                        }
                    }
                    _ => {}
                }
            }
        })?;
        self.alvos = alvos;
        self.sujo = false;
        Ok(())
    }
}

/// A barra ocupa o topo do seu monitor e reserva a própria altura:
/// janelas maximizadas começam abaixo dela.
fn posicionar_janela(hwnd: HWND, reservar: bool, monitor: HMONITOR, ultima: &mut Option<RECT>) {
    let e = janela::escala(hwnd);
    let ap = (ALTURA * e).ceil() as i32;
    let (mon, trabalho) = janela::info_monitor(monitor);
    let topo = if reservar {
        janela::reservar_appbar(hwnd, mon, true, ap, ultima).top
    } else {
        trabalho.top
    };
    janela::posicionar(hwnd, mon.left, topo, mon.right - mon.left, ap);
}

/// Win + tecla (abre os painéis do próprio Windows).
pub fn teclas_windows(tecla: VIRTUAL_KEY) {
    let k = |vk: VIRTUAL_KEY, solta: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: if solta { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                ..Default::default()
            },
        },
    };
    let seq = [k(VK_LWIN, false), k(tecla, false), k(tecla, true), k(VK_LWIN, true)];
    unsafe {
        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Títulos da barra de menus clássica do app (Arquivo, Editar...), se houver.
fn titulos_de_menu(h: HWND) -> Vec<String> {
    unsafe {
        let m = GetMenu(h);
        if m.0.is_null() {
            return Vec::new();
        }
        let n = GetMenuItemCount(m).clamp(0, 10);
        (0..n)
            .filter_map(|i| {
                let mut buf = [0u16; 128];
                let len = GetMenuStringW(m, i as u32, Some(&mut buf), MF_BYPOSITION) as usize;
                let t = String::from_utf16_lossy(&buf[..len.min(buf.len())])
                    .replace('&', "")
                    .trim()
                    .to_string();
                (!t.is_empty()).then_some(t)
            })
            .collect()
    }
}

/// Copia um submenu do app para um menu nosso e, se você escolher um item,
/// manda o comando para o app (como se você tivesse clicado no menu dele).
fn menu_classico(nosso: HWND, app: HWND, indice: i32, x: i32, y: i32) {
    unsafe {
        let barra = GetMenu(app);
        if barra.0.is_null() {
            return;
        }
        let sub = GetSubMenu(barra, indice);
        if sub.0.is_null() {
            return;
        }
        // Deixa o app atualizar o estado dos itens (ativos, marcados).
        SendMessageTimeoutW(
            app,
            WM_INITMENUPOPUP,
            WPARAM(sub.0 as usize),
            LPARAM(indice as isize),
            SMTO_ABORTIFHUNG,
            150,
            None,
        );
        let Ok(m) = CreatePopupMenu() else { return };
        copiar_menu(sub, m, 0);
        let _ = SetForegroundWindow(nosso);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, x, y, 0, nosso, None).0;
        let _ = DestroyMenu(m);
        let _ = SetForegroundWindow(app);
        if cmd != 0 {
            let _ = PostMessageW(app, WM_COMMAND, WPARAM(cmd as usize & 0xffff), LPARAM(0));
        }
    }
}

unsafe fn copiar_menu(origem: HMENU, destino: HMENU, profundidade: u32) {
    let n = GetMenuItemCount(origem);
    for i in 0..n.max(0) as u32 {
        let mut info = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_SUBMENU,
            ..Default::default()
        };
        if GetMenuItemInfoW(origem, i, true, &mut info).is_err() {
            continue;
        }
        if info.fType.0 & MFT_SEPARATOR.0 != 0 {
            let _ = AppendMenuW(destino, MF_SEPARATOR, 0, PCWSTR::null());
            continue;
        }
        let mut buf = [0u16; 256];
        let len = GetMenuStringW(origem, i, Some(&mut buf[..255]), MF_BYPOSITION);
        if len <= 0 {
            continue;
        }
        let mut flags = MF_STRING;
        if info.fState.0 & MFS_DISABLED.0 != 0 {
            flags |= MF_GRAYED;
        }
        if info.fState.0 & MFS_CHECKED.0 != 0 {
            flags |= MF_CHECKED;
        }
        let texto = PCWSTR(buf.as_ptr());
        if !info.hSubMenu.0.is_null() && profundidade < 3 {
            if let Ok(sub) = CreatePopupMenu() {
                copiar_menu(info.hSubMenu, sub, profundidade + 1);
                let _ = AppendMenuW(destino, flags | MF_POPUP, sub.0 as usize, texto);
            }
        } else {
            let _ = AppendMenuW(destino, flags, info.wID as usize, texto);
        }
    }
}

/// Largura de cada ponto das áreas de trabalho virtuais.
const LARG_AREA: f32 = 14.0;

/// Programas "hospedeiros" (Electron, Java...): a barra usa o título da
/// janela, porque o nome do .exe não diz qual é o app.
fn nome_pelo_titulo(exe: &str, h: HWND) -> Option<String> {
    if !apps::eh_hospedeiro(exe) {
        return None;
    }
    apps::nome_pelo_titulo(h)
}

// Ícones desenhados à mão (nada de ícones de terceiros).

unsafe fn icone_logo(dc: &ID2D1DeviceContext, p: &ID2D1SolidColorBrush, cx: f32, cy: f32) {
    // Um "cais": a base de um dock com três ícones em cima.
    p.SetColor(&crate::tema::tinta(0.95));
    dc.FillRoundedRectangle(
        &arredondado(retangulo(cx - 8.0, cy + 3.0, cx + 8.0, cy + 6.5), 1.75),
        p,
    );
    for (i, a) in [(-1.0f32, 5.0f32), (0.0, 8.0), (1.0, 5.0)] {
        let x = cx + i * 5.0;
        dc.FillRoundedRectangle(
            &arredondado(retangulo(x - 2.0, cy + 1.5 - a, x + 2.0, cy + 1.5), 1.2),
            p,
        );
    }
}

unsafe fn icone_busca(dc: &ID2D1DeviceContext, p: &ID2D1SolidColorBrush, cx: f32, cy: f32) {
    p.SetColor(&crate::tema::tinta(0.94));
    dc.DrawEllipse(
        &D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: cx - 1.5, y: cy - 1.5 },
            radiusX: 5.0,
            radiusY: 5.0,
        },
        p,
        1.6,
        None,
    );
    dc.DrawLine(
        D2D_POINT_2F { x: cx + 2.2, y: cy + 2.2 },
        D2D_POINT_2F { x: cx + 6.0, y: cy + 6.0 },
        p,
        1.8,
        None,
    );
}

unsafe fn icone_controle(dc: &ID2D1DeviceContext, p: &ID2D1SolidColorBrush, cx: f32, cy: f32) {
    p.SetColor(&crate::tema::tinta(0.94));
    for (dy, bola) in [(-3.6f32, 1.0f32), (3.6, -1.0)] {
        let y = cy + dy;
        dc.DrawRoundedRectangle(
            &arredondado(retangulo(cx - 7.5, y - 2.6, cx + 7.5, y + 2.6), 2.6),
            p,
            1.2,
            None,
        );
        dc.FillEllipse(
            &D2D1_ELLIPSE {
                point: D2D_POINT_2F { x: cx + bola * 4.9, y },
                radiusX: 1.9,
                radiusY: 1.9,
            },
            p,
        );
    }
}

unsafe fn icone_bateria(
    dc: &ID2D1DeviceContext,
    p: &ID2D1SolidColorBrush,
    x: f32,
    cy: f32,
    pct: u8,
    carregando: bool,
) {
    p.SetColor(&crate::tema::tinta(0.55));
    dc.DrawRoundedRectangle(
        &arredondado(retangulo(x, cy - 5.5, x + 22.0, cy + 5.5), 3.0),
        p,
        1.1,
        None,
    );
    dc.FillRoundedRectangle(
        &arredondado(retangulo(x + 23.0, cy - 2.0, x + 24.6, cy + 2.0), 0.8),
        p,
    );
    let fracao = pct as f32 / 100.0;
    let baixa = pct <= 20 && !carregando;
    p.SetColor(&if baixa {
        cor(1.0, 0.36, 0.33, 1.0)
    } else if carregando {
        cor(0.36, 0.86, 0.45, 1.0)
    } else {
        crate::tema::tinta(0.94)
    });
    dc.FillRoundedRectangle(
        &arredondado(
            retangulo(x + 1.8, cy - 3.7, x + 1.8 + 18.4 * fracao, cy + 3.7),
            1.8,
        ),
        p,
    );
}
