//! Estado do sistema para a Central de Controle e a barra do topo: Wi-Fi,
//! Bluetooth, rede, som (geral, por app e saída), mídia tocando e brilho.
//!
//! Tudo roda numa thread própria que dorme esperando comandos. Quem acorda
//! a thread são avisos do próprio Windows (o volume mudou, a rede caiu, a
//! música trocou...) ou um clique seu no painel. Nada é consultado em
//! intervalos, e nada sai do computador: só lemos o estado local.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use windows::core::{implement, Interface, Result, GUID, PCWSTR};
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Media::Control::*;
use windows::Networking::Connectivity::{NetworkInformation, NetworkStatusChangedEventHandler};
use windows::Win32::Devices::Display::*;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::Media::Audio::Endpoints::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// "O estado do sistema mudou": vai para a janela da barra do topo.
pub const WM_SISTEMA: u32 = WM_APP + 4;

#[derive(Clone, Default, PartialEq)]
pub enum Rede {
    #[default]
    Nenhuma,
    Wifi { nome: String, barras: u8 },
    Cabo { nome: String },
}

#[derive(Clone, Default, PartialEq)]
pub struct Saida {
    pub id: String,
    pub nome: String,
}

#[derive(Clone, Default, PartialEq)]
pub struct SomDoApp {
    /// Caminho do executável (vazio = sons do sistema).
    pub exe: String,
    pub nome: String,
    pub volume: f32,
    pub mudo: bool,
}

#[derive(Clone, Default, PartialEq)]
pub struct Midia {
    pub titulo: String,
    pub artista: String,
    pub app: String,
    pub tocando: bool,
}

#[derive(Clone, Default, PartialEq)]
pub struct Estado {
    /// None = este PC não tem o rádio.
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
    pub rede: Rede,
    pub volume: f32,
    pub mudo: bool,
    pub tem_som: bool,
    pub saida_atual: String,
    pub saidas: Vec<Saida>,
    pub apps: Vec<SomDoApp>,
    pub midia: Option<Midia>,
    /// 0 a 100. None = o monitor não deixa ajustar (ou ainda não lido).
    pub brilho: Option<u32>,
}

pub enum Cmd {
    /// Liga tudo. Mandado depois do primeiro quadro, para não disputar a
    /// abertura do Cais com o carregamento das bibliotecas do Windows.
    Comecar,
    Rede,
    Radios,
    Audio,
    Saidas,
    Midia,
    NovaSessaoDeMidia,
    Painel(bool),
    LigarWifi(bool),
    LigarBluetooth(bool),
    Volume(f32),
    Mudo(bool),
    UsarSaida(String),
    VolumeApp(String, f32),
    MudoApp(String, bool),
    DefinirBrilho(u32),
    TocarPausar,
    Proxima,
    Anterior,
    Sair,
}

impl Cmd {
    /// Comandos repetidos na fila: basta executar o último (ex.: arrastar o
    /// volume gera dezenas de pedidos, só o valor final importa).
    fn mesmo_tipo(&self, outro: &Cmd) -> bool {
        use Cmd::*;
        match (self, outro) {
            (VolumeApp(a, _), VolumeApp(b, _)) => a == b,
            (Volume(_), Volume(_)) | (DefinirBrilho(_), DefinirBrilho(_)) => true,
            (Rede, Rede) | (Radios, Radios) | (Audio, Audio) | (Saidas, Saidas) => true,
            (Midia, Midia) | (NovaSessaoDeMidia, NovaSessaoDeMidia) => true,
            _ => false,
        }
    }
}

/// Avisa a janela da barra, sem empilhar mensagens repetidas.
#[derive(Clone)]
struct Aviso {
    hwnd: isize,
    pendente: Arc<AtomicBool>,
}

impl Aviso {
    fn avisar(&self) {
        if !self.pendente.swap(true, Ordering::AcqRel) {
            unsafe {
                let _ = PostMessageW(HWND(self.hwnd as *mut _), WM_SISTEMA, WPARAM(0), LPARAM(0));
            }
        }
    }
}

pub struct Sistema {
    tx: Sender<Cmd>,
    estado: Arc<Mutex<Estado>>,
    pendente: Arc<AtomicBool>,
}

impl Sistema {
    pub fn iniciar(hwnd_aviso: HWND) -> Self {
        let (tx, rx) = channel();
        let estado = Arc::new(Mutex::new(Estado::default()));
        let pendente = Arc::new(AtomicBool::new(false));
        let aviso = Aviso {
            hwnd: hwnd_aviso.0 as isize,
            pendente: pendente.clone(),
        };
        let (e, t) = (estado.clone(), tx.clone());
        let _ = std::thread::Builder::new()
            .name("cais-sistema".into())
            .stack_size(512 * 1024)
            .spawn(move || trabalhar(rx, t, e, aviso));
        Self {
            tx,
            estado,
            pendente,
        }
    }

    pub fn remetente(&self) -> Sender<Cmd> {
        self.tx.clone()
    }

    /// Lê o estado atual (e libera o próximo aviso).
    pub fn ler(&self) -> Estado {
        self.pendente.store(false, Ordering::Release);
        self.estado.lock().map(|e| e.clone()).unwrap_or_default()
    }

    /// Pede para a thread soltar os avisos do Windows. Não esperamos por ela:
    /// ao sair do processo ela termina junto.
    pub fn encerrar(&self) {
        let _ = self.tx.send(Cmd::Sair);
    }
}

// ---------------------------------------------------------------------------
// Avisos COM do som

#[implement(IAudioEndpointVolumeCallback)]
struct AvisoVolume {
    estado: Arc<Mutex<Estado>>,
    aviso: Aviso,
}

impl IAudioEndpointVolumeCallback_Impl for AvisoVolume_Impl {
    fn OnNotify(&self, p: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        if let Some(d) = unsafe { p.as_ref() } {
            if let Ok(mut e) = self.estado.lock() {
                e.volume = d.fMasterVolume;
                e.mudo = d.bMuted.as_bool();
            }
            self.aviso.avisar();
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct AvisoDispositivos {
    tx: Sender<Cmd>,
}

impl IMMNotificationClient_Impl for AvisoDispositivos_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        let _ = self.tx.send(Cmd::Saidas);
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        let _ = self.tx.send(Cmd::Saidas);
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        let _ = self.tx.send(Cmd::Saidas);
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, fluxo: EDataFlow, papel: ERole, _: &PCWSTR) -> Result<()> {
        if fluxo == eRender && papel == eConsole {
            let _ = self.tx.send(Cmd::Audio);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// Interface (não documentada, mas estável desde o Windows 7) que o próprio
/// Windows usa para trocar o dispositivo de som padrão.
#[allow(non_snake_case)]
mod politica {
use windows::core::{interface, IUnknown, IUnknown_Vtbl, HRESULT, PCWSTR};
use windows::Win32::Media::Audio::ERole;
#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
pub unsafe trait IPolicyConfig: IUnknown {
    fn GetMixFormat(&self, id: PCWSTR, f: *mut *mut core::ffi::c_void) -> HRESULT;
    fn GetDeviceFormat(&self, id: PCWSTR, d: i32, f: *mut *mut core::ffi::c_void) -> HRESULT;
    fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(&self, id: PCWSTR, a: *mut core::ffi::c_void, b: *mut core::ffi::c_void) -> HRESULT;
    fn GetProcessingPeriod(&self, id: PCWSTR, d: i32, a: *mut i64, b: *mut i64) -> HRESULT;
    fn SetProcessingPeriod(&self, id: PCWSTR, a: *mut i64) -> HRESULT;
    fn GetShareMode(&self, id: PCWSTR, m: *mut core::ffi::c_void) -> HRESULT;
    fn SetShareMode(&self, id: PCWSTR, m: *mut core::ffi::c_void) -> HRESULT;
    fn GetPropertyValue(&self, id: PCWSTR, k: *const core::ffi::c_void, v: *mut core::ffi::c_void) -> HRESULT;
    fn SetPropertyValue(&self, id: PCWSTR, k: *const core::ffi::c_void, v: *mut core::ffi::c_void) -> HRESULT;
    fn SetDefaultEndpoint(&self, id: PCWSTR, papel: ERole) -> HRESULT;
    fn SetEndpointVisibility(&self, id: PCWSTR, visivel: i32) -> HRESULT;
}
pub unsafe fn definir_padrao(pc: &IPolicyConfig, id: PCWSTR, papel: ERole) -> HRESULT {
    pc.SetDefaultEndpoint(id, papel)
}
}
use politica::IPolicyConfig;
const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

// ---------------------------------------------------------------------------
// A thread

struct Trabalho {
    estado: Arc<Mutex<Estado>>,
    aviso: Aviso,
    tx: Sender<Cmd>,
    painel_aberto: bool,
    radios: Vec<Radio>,
    enumerador: Option<IMMDeviceEnumerator>,
    aviso_dispositivos: Option<IMMNotificationClient>,
    endpoint: Option<IAudioEndpointVolume>,
    aviso_volume: IAudioEndpointVolumeCallback,
    sessoes: Vec<(String, ISimpleAudioVolume)>,
    midia: Option<GlobalSystemMediaTransportControlsSessionManager>,
    sessao_midia: Option<(GlobalSystemMediaTransportControlsSession, EventRegistrationToken, EventRegistrationToken)>,
    monitor: Option<(HANDLE, u32, u32)>,
    monitor_lido: bool,
}

fn trabalhar(rx: Receiver<Cmd>, tx: Sender<Cmd>, estado: Arc<Mutex<Estado>>, aviso: Aviso) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let aviso_volume: IAudioEndpointVolumeCallback = AvisoVolume {
        estado: estado.clone(),
        aviso: aviso.clone(),
    }
    .into();
    // Espera o sinal para começar (a tela do Cais abre primeiro).
    loop {
        match rx.recv() {
            Ok(Cmd::Comecar) => break,
            Ok(Cmd::Sair) | Err(_) => return,
            Ok(_) => {}
        }
    }
    let mut t = Trabalho {
        estado,
        aviso,
        tx,
        painel_aberto: false,
        radios: Vec::new(),
        enumerador: None,
        aviso_dispositivos: None,
        endpoint: None,
        aviso_volume,
        sessoes: Vec::new(),
        midia: None,
        sessao_midia: None,
        monitor: None,
        monitor_lido: false,
    };
    t.iniciar_som();
    t.iniciar_rede();
    t.iniciar_radios();
    t.iniciar_midia();
    t.publicar();
    // O que foi carregado só para subir já pode sair da memória.
    crate::medidor::enxugar_memoria();

    while let Ok(primeiro) = rx.recv() {
        let mut fila = vec![primeiro];
        while let Ok(c) = rx.try_recv() {
            fila.push(c);
        }
        let mut sair = false;
        for i in 0..fila.len() {
            if fila[i + 1..].iter().any(|d| fila[i].mesmo_tipo(d)) {
                continue;
            }
            if matches!(fila[i], Cmd::Sair) {
                sair = true;
                break;
            }
            t.executar(&fila[i]);
        }
        if sair {
            break;
        }
        t.publicar();
    }
    t.encerrar();
}

impl Trabalho {
    fn mudar(&self, f: impl FnOnce(&mut Estado)) {
        if let Ok(mut e) = self.estado.lock() {
            f(&mut e);
        }
    }

    fn publicar(&self) {
        self.aviso.avisar();
    }

    fn executar(&mut self, c: &Cmd) {
        match c {
            Cmd::Rede => self.ler_rede(),
            Cmd::Radios => self.ler_radios(),
            Cmd::Audio => {
                self.ligar_saida_padrao();
                if self.painel_aberto {
                    self.ler_sessoes();
                }
            }
            Cmd::Saidas => {
                self.ler_saidas();
                self.ligar_saida_padrao();
            }
            Cmd::Midia => self.ler_midia(),
            Cmd::NovaSessaoDeMidia => self.ligar_sessao_de_midia(),
            Cmd::Painel(aberto) => {
                self.painel_aberto = *aberto;
                if *aberto {
                    self.ler_saidas();
                    self.ler_sessoes();
                    self.ler_midia();
                    self.ler_brilho();
                } else {
                    // Sem painel não precisamos segurar as sessões de som.
                    self.sessoes.clear();
                    self.mudar(|e| e.apps.clear());
                }
            }
            Cmd::LigarWifi(v) => self.ligar_radio(RadioKind::WiFi, *v),
            Cmd::LigarBluetooth(v) => self.ligar_radio(RadioKind::Bluetooth, *v),
            Cmd::Volume(v) => unsafe {
                if let Some(ep) = &self.endpoint {
                    let _ = ep.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null());
                    if *v > 0.0 {
                        let _ = ep.SetMute(false, std::ptr::null());
                    }
                }
            },
            Cmd::Mudo(m) => unsafe {
                if let Some(ep) = &self.endpoint {
                    let _ = ep.SetMute(*m, std::ptr::null());
                }
            },
            Cmd::UsarSaida(id) => self.usar_saida(id),
            Cmd::VolumeApp(exe, v) => {
                for (e, s) in &self.sessoes {
                    if e == exe {
                        unsafe {
                            let _ = s.SetMasterVolume(v.clamp(0.0, 1.0), std::ptr::null());
                            if *v > 0.0 {
                                let _ = s.SetMute(false, std::ptr::null());
                            }
                        }
                    }
                }
                let v = *v;
                self.mudar(|e| {
                    if let Some(a) = e.apps.iter_mut().find(|a| a.exe == *exe) {
                        a.volume = v;
                        if v > 0.0 {
                            a.mudo = false;
                        }
                    }
                });
            }
            Cmd::MudoApp(exe, m) => {
                for (e, s) in &self.sessoes {
                    if e == exe {
                        unsafe {
                            let _ = s.SetMute(*m, std::ptr::null());
                        }
                    }
                }
                let m = *m;
                self.mudar(|e| {
                    if let Some(a) = e.apps.iter_mut().find(|a| a.exe == *exe) {
                        a.mudo = m;
                    }
                });
            }
            Cmd::DefinirBrilho(p) => self.definir_brilho(*p),
            Cmd::TocarPausar | Cmd::Proxima | Cmd::Anterior => self.controlar_midia(c),
            Cmd::Sair | Cmd::Comecar => {}
        }
    }

    fn encerrar(&mut self) {
        unsafe {
            if let Some(ep) = &self.endpoint {
                let _ = ep.UnregisterControlChangeNotify(&self.aviso_volume);
            }
            if let (Some(en), Some(a)) = (&self.enumerador, &self.aviso_dispositivos) {
                let _ = en.UnregisterEndpointNotificationCallback(a);
            }
            if let Some((h, _, _)) = self.monitor.take() {
                let _ = DestroyPhysicalMonitor(h);
            }
        }
        self.sessao_midia = None;
        self.midia = None;
        self.radios.clear();
    }

    // ----- rede -----

    fn iniciar_rede(&mut self) {
        let tx = self.tx.clone();
        let _ = NetworkInformation::NetworkStatusChanged(&NetworkStatusChangedEventHandler::new(
            move |_| {
                let _ = tx.send(Cmd::Rede);
                Ok(())
            },
        ));
        self.ler_rede();
    }

    fn ler_rede(&mut self) {
        let rede = (|| -> Result<Rede> {
            let p = NetworkInformation::GetInternetConnectionProfile()?;
            // O nome do perfil é o nome da rede; não pedimos o SSID direto
            // porque isso exige permissão de localização no Windows 11.
            let nome = p.ProfileName().map(|s| s.to_string()).unwrap_or_default();
            if p.IsWlanConnectionProfile().unwrap_or(false) {
                let barras = p.GetSignalBars().and_then(|b| b.Value()).unwrap_or(3);
                Ok(Rede::Wifi { nome, barras })
            } else {
                Ok(Rede::Cabo { nome })
            }
        })()
        .unwrap_or(Rede::Nenhuma);
        self.mudar(|e| e.rede = rede);
    }

    // ----- rádios (Wi-Fi e Bluetooth) -----

    fn iniciar_radios(&mut self) {
        let lista = (|| -> Result<Vec<Radio>> {
            let _ = Radio::RequestAccessAsync()?.get();
            let v = Radio::GetRadiosAsync()?.get()?;
            let mut r = Vec::new();
            for i in 0..v.Size()? {
                r.push(v.GetAt(i)?);
            }
            Ok(r)
        })()
        .unwrap_or_default();
        for r in &lista {
            let tx = self.tx.clone();
            let _ = r.StateChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Cmd::Radios);
                Ok(())
            }));
        }
        self.radios = lista;
        self.ler_radios();
    }

    fn radio(&self, tipo: RadioKind) -> Option<&Radio> {
        self.radios.iter().find(|r| r.Kind().ok() == Some(tipo))
    }

    fn ler_radios(&mut self) {
        let ligado = |r: Option<&Radio>| r.map(|r| r.State().ok() == Some(RadioState::On));
        let wifi = ligado(self.radio(RadioKind::WiFi));
        let bt = ligado(self.radio(RadioKind::Bluetooth));
        self.mudar(|e| {
            e.wifi = wifi;
            e.bluetooth = bt;
        });
    }

    fn ligar_radio(&mut self, tipo: RadioKind, ligar: bool) {
        if let Some(r) = self.radio(tipo) {
            let alvo = if ligar { RadioState::On } else { RadioState::Off };
            if let Ok(op) = r.SetStateAsync(alvo) {
                let _ = op.get();
            }
        }
        self.ler_radios();
    }

    // ----- som -----

    fn iniciar_som(&mut self) {
        unsafe {
            let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
            else {
                return;
            };
            let aviso: IMMNotificationClient = AvisoDispositivos { tx: self.tx.clone() }.into();
            let _ = en.RegisterEndpointNotificationCallback(&aviso);
            self.enumerador = Some(en);
            self.aviso_dispositivos = Some(aviso);
        }
        self.ler_saidas();
        self.ligar_saida_padrao();
    }

    /// Passa a ouvir o volume do dispositivo de saída padrão (de novo,
    /// sempre que o padrão muda).
    fn ligar_saida_padrao(&mut self) {
        unsafe {
            if let Some(ep) = self.endpoint.take() {
                let _ = ep.UnregisterControlChangeNotify(&self.aviso_volume);
            }
            let Some(en) = &self.enumerador else { return };
            let dispositivo = en.GetDefaultAudioEndpoint(eRender, eConsole).ok();
            let ep = dispositivo
                .as_ref()
                .and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok());
            let id = dispositivo
                .as_ref()
                .and_then(|d| d.GetId().ok())
                .map(|p| {
                    let s = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as _));
                    s
                })
                .unwrap_or_default();
            let (vol, mudo) = match &ep {
                Some(ep) => {
                    let _ = ep.RegisterControlChangeNotify(&self.aviso_volume);
                    (
                        ep.GetMasterVolumeLevelScalar().unwrap_or(0.0),
                        ep.GetMute().map(|b| b.as_bool()).unwrap_or(false),
                    )
                }
                None => (0.0, false),
            };
            let tem = ep.is_some();
            self.endpoint = ep;
            self.mudar(|e| {
                e.tem_som = tem;
                e.volume = vol;
                e.mudo = mudo;
                e.saida_atual = id;
            });
        }
    }

    fn ler_saidas(&mut self) {
        let Some(en) = &self.enumerador else { return };
        let mut v = Vec::new();
        unsafe {
            if let Ok(lista) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
                for i in 0..lista.GetCount().unwrap_or(0) {
                    let Ok(d) = lista.Item(i) else { continue };
                    let Ok(p) = d.GetId() else { continue };
                    let id = p.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(p.0 as _));
                    let nome = d
                        .OpenPropertyStore(STGM_READ)
                        .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
                        .map(|p| p.to_string())
                        .unwrap_or_default();
                    v.push(Saida { id, nome });
                }
            }
        }
        self.mudar(|e| e.saidas = v);
    }

    fn usar_saida(&mut self, id: &str) {
        let w: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
        unsafe {
            if let Ok(pc) = CoCreateInstance::<_, IPolicyConfig>(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) {
                for papel in [eConsole, eMultimedia, eCommunications] {
                    let _ = politica::definir_padrao(&pc, PCWSTR(w.as_ptr()), papel);
                }
            }
        }
        // O aviso OnDefaultDeviceChanged chega em seguida e religa o volume.
    }

    /// Lista os apps que estão usando o som agora (um item por programa).
    fn ler_sessoes(&mut self) {
        self.sessoes.clear();
        let mut apps: Vec<SomDoApp> = Vec::new();
        unsafe {
            let Some(en) = &self.enumerador else { return };
            let Ok(d) = en.GetDefaultAudioEndpoint(eRender, eConsole) else { return };
            let Ok(gerente) = d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                return;
            };
            let Ok(lista) = gerente.GetSessionEnumerator() else { return };
            for i in 0..lista.GetCount().unwrap_or(0) {
                let Ok(c) = lista.GetSession(i) else { continue };
                let Ok(c2) = c.cast::<IAudioSessionControl2>() else { continue };
                if c.GetState().ok() == Some(AudioSessionStateExpired) {
                    continue;
                }
                let Ok(vol) = c.cast::<ISimpleAudioVolume>() else { continue };
                let sistema = c2.IsSystemSoundsSession() == S_OK;
                let exe = if sistema {
                    String::new()
                } else {
                    let pid = c2.GetProcessId().unwrap_or(0);
                    match crate::apps::exe_do_pid(pid) {
                        Some(e) => e,
                        None => continue,
                    }
                };
                if !apps.iter().any(|a| a.exe == exe) {
                    let nome = if sistema {
                        crate::idioma::t("Sons do sistema", "System sounds").to_string()
                    } else {
                        crate::apps::nome_do_exe(&exe)
                    };
                    apps.push(SomDoApp {
                        exe: exe.clone(),
                        nome,
                        volume: vol.GetMasterVolume().unwrap_or(1.0),
                        mudo: vol.GetMute().map(|b| b.as_bool()).unwrap_or(false),
                    });
                }
                self.sessoes.push((exe, vol));
            }
        }
        // Sons do sistema por último, o resto em ordem alfabética.
        apps.sort_by_key(|a| (a.exe.is_empty(), a.nome.to_lowercase()));
        self.mudar(|e| e.apps = apps);
    }

    // ----- mídia tocando -----

    fn iniciar_midia(&mut self) {
        let Ok(op) = GlobalSystemMediaTransportControlsSessionManager::RequestAsync() else {
            return;
        };
        let Ok(m) = op.get() else { return };
        let tx = self.tx.clone();
        let _ = m.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(Cmd::NovaSessaoDeMidia);
            Ok(())
        }));
        self.midia = Some(m);
        self.ligar_sessao_de_midia();
    }

    fn ligar_sessao_de_midia(&mut self) {
        if let Some((s, a, b)) = self.sessao_midia.take() {
            let _ = s.RemoveMediaPropertiesChanged(a);
            let _ = s.RemovePlaybackInfoChanged(b);
        }
        let sessao = self.midia.as_ref().and_then(|m| m.GetCurrentSession().ok());
        if let Some(s) = sessao {
            let (t1, t2) = (self.tx.clone(), self.tx.clone());
            let a = s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = t1.send(Cmd::Midia);
                Ok(())
            }));
            let b = s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = t2.send(Cmd::Midia);
                Ok(())
            }));
            if let (Ok(a), Ok(b)) = (a, b) {
                self.sessao_midia = Some((s, a, b));
            }
        }
        self.ler_midia();
    }

    fn ler_midia(&mut self) {
        let midia = self.sessao_midia.as_ref().and_then(|(s, _, _)| {
            let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
            let titulo = props.Title().map(|t| t.to_string()).unwrap_or_default();
            if titulo.is_empty() {
                return None;
            }
            let tocando = s
                .GetPlaybackInfo()
                .and_then(|i| i.PlaybackStatus())
                .map(|p| p == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
                .unwrap_or(false);
            Some(Midia {
                titulo,
                artista: props.Artist().map(|t| t.to_string()).unwrap_or_default(),
                app: nome_do_app_de_midia(&s.SourceAppUserModelId().map(|t| t.to_string()).unwrap_or_default()),
                tocando,
            })
        });
        self.mudar(|e| e.midia = midia);
    }

    fn controlar_midia(&mut self, c: &Cmd) {
        if let Some((s, _, _)) = &self.sessao_midia {
            let op = match c {
                Cmd::TocarPausar => s.TryTogglePlayPauseAsync(),
                Cmd::Proxima => s.TrySkipNextAsync(),
                _ => s.TrySkipPreviousAsync(),
            };
            if let Ok(op) = op {
                let _ = op.get();
            }
        }
    }

    // ----- brilho (DDC/CI do monitor principal) -----

    fn abrir_monitor(&mut self) {
        if self.monitor_lido {
            return;
        }
        self.monitor_lido = true;
        unsafe {
            let hmon = MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY);
            let mut n = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(hmon, &mut n).is_err() || n == 0 {
                return;
            }
            let mut fisicos = vec![PHYSICAL_MONITOR::default(); n as usize];
            if GetPhysicalMonitorsFromHMONITOR(hmon, &mut fisicos).is_err() {
                return;
            }
            let h = fisicos[0].hPhysicalMonitor;
            for f in &fisicos[1..] {
                let _ = DestroyPhysicalMonitor(f.hPhysicalMonitor);
            }
            let (mut min, mut atual, mut max) = (0u32, 0u32, 0u32);
            if GetMonitorBrightness(h, &mut min, &mut atual, &mut max) != 0 && max > min {
                self.monitor = Some((h, min, max));
            } else {
                let _ = DestroyPhysicalMonitor(h);
            }
        }
    }

    fn ler_brilho(&mut self) {
        self.abrir_monitor();
        let Some((h, min, max)) = self.monitor else { return };
        let (mut a, mut b, mut c) = (0u32, 0u32, 0u32);
        let ok = unsafe { GetMonitorBrightness(h, &mut a, &mut b, &mut c) } != 0;
        if ok {
            let pct = ((b.saturating_sub(min)) * 100 + (max - min) / 2) / (max - min);
            self.mudar(|e| e.brilho = Some(pct.min(100)));
        }
    }

    fn definir_brilho(&mut self, pct: u32) {
        let Some((h, min, max)) = self.monitor else { return };
        let valor = min + (max - min) * pct.min(100) / 100;
        unsafe {
            let _ = SetMonitorBrightness(h, valor);
        }
        self.mudar(|e| e.brilho = Some(pct.min(100)));
    }
}

/// "Spotify.exe" -> "Spotify"; "SpotifyAB.SpotifyMusic_xxx!Spotify" -> "Spotify".
fn nome_do_app_de_midia(aumid: &str) -> String {
    let base = aumid.rsplit('!').next().unwrap_or(aumid);
    let base = base.rsplit('\\').next().unwrap_or(base);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let nome = match base.to_lowercase().as_str() {
        "msedge" | "microsoftedge" => "Microsoft Edge".to_string(),
        "chrome" => "Google Chrome".to_string(),
        "firefox" => "Firefox".to_string(),
        "app" => crate::idioma::t("Música", "Music").to_string(),
        _ => base.to_string(),
    };
    let mut c = nome.chars();
    match c.next() {
        Some(p) => p.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
