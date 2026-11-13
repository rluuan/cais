//! Instalar e desinstalar, sem programa de instalação à parte: o próprio
//! cais.exe se copia para %LOCALAPPDATA%\Programs\Cais (sem pedir
//! administrador), põe um atalho no menu Iniciar e aparece em
//! Configurações > Aplicativos instalados, de onde dá para desinstalar.
//! Nada é baixado: tudo vem do arquivo que você já tem.

use std::path::{Path, PathBuf};
use windows::core::{w, Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const CHAVE_DESINSTALAR: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Cais");
const CHAVE_RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const MUTEX: PCWSTR = w!("Local\\Cais.Instancia");
/// Mensagem que pede para o Cais aberto sair (o desinstalador manda).
pub const MSG_SAIR: PCWSTR = w!("Cais.Sair");

// ---------------------------------------------------------------------------
// Abrir como administrador ao entrar: uma tarefa agendada do Windows
// ("Cais (administrador)") que abre o Cais com privilégios elevados no logon.
// Criar e apagar a tarefa pede confirmação do Windows (UAC).

const TAREFA_ADMIN: &str = "Cais (administrador)";
static ADMIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Avisa a janela `aviso` quando a tarefa foi criada ou apagada.
pub const WM_ADMIN: u32 = WM_APP + 33;

pub fn admin_ligado() -> bool {
    ADMIN.load(std::sync::atomic::Ordering::Relaxed)
}

/// Confere (numa thread) se a tarefa existe.
pub fn conferir_admin() {
    std::thread::spawn(|| {
        use std::os::windows::process::CommandExt;
        let existe = std::process::Command::new(r"C:\Windows\System32\schtasks.exe")
            .args(["/Query", "/TN", TAREFA_ADMIN])
            .creation_flags(0x0800_0000) // sem janela de console
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        ADMIN.store(existe, std::sync::atomic::Ordering::Relaxed);
    });
}

/// Cria ou apaga a tarefa (o Windows pede confirmação). Avisa `aviso` no fim.
pub fn alternar_admin(aviso: HWND) {
    let ligar = !admin_ligado();
    let exe = crate::config::caminho_do_cais().to_string_lossy().into_owned();
    let h = aviso.0 as isize;
    std::thread::spawn(move || unsafe {
        let parametros = if ligar {
            format!("/Create /F /TN \"{TAREFA_ADMIN}\" /SC ONLOGON /RL HIGHEST /TR \"\\\"{exe}\\\"\"")
        } else {
            format!("/Delete /F /TN \"{TAREFA_ADMIN}\"")
        };
        let p = HSTRING::from(parametros);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: w!("runas"),
            lpFile: w!(r"C:\Windows\System32\schtasks.exe"),
            lpParameters: PCWSTR(p.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        if ShellExecuteExW(&mut info).is_ok() && !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, 60_000);
            let mut codigo = 1u32;
            let _ = GetExitCodeProcess(info.hProcess, &mut codigo);
            let _ = CloseHandle(info.hProcess);
            if codigo == 0 {
                ADMIN.store(ligar, std::sync::atomic::Ordering::Relaxed);
                if ligar {
                    // A tarefa abre o Cais; a chave "Run" abriria um segundo.
                    crate::config::definir_inicio_com_windows(false);
                }
            }
        }
        let _ = PostMessageW(HWND(h as *mut _), WM_ADMIN, WPARAM(0), LPARAM(0));
    });
}

pub fn pasta_instalacao() -> PathBuf {
    PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join(r"Programs\Cais")
}

pub fn exe_instalado() -> PathBuf {
    pasta_instalacao().join("cais.exe")
}

fn atalho_do_menu() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_default()).join(r"Microsoft\Windows\Start Menu\Programs\Cais.lnk")
}

fn mesmo_caminho(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// Este processo está rodando da pasta de instalação?
pub fn rodando_instalado() -> bool {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())).is_some_and(|p| mesmo_caminho(&p, &pasta_instalacao()))
}

// ----------------------------------------------------------------------
// Uma instância só

pub struct Unica(HANDLE);

impl Drop for Unica {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Garante que só um Cais rode por vez. None = já tem um aberto.
pub fn uma_instancia() -> Option<Unica> {
    unsafe {
        let h = CreateMutexW(None, true, MUTEX);
        let erro = GetLastError();
        match h {
            Ok(h) if erro != ERROR_ALREADY_EXISTS => Some(Unica(h)),
            Ok(h) => {
                let _ = CloseHandle(h);
                None
            }
            // Acesso negado: o mutex existe, de um Cais rodando como administrador.
            Err(_) => None,
        }
    }
}

/// Pede para o Cais aberto sair e espera até 5 s.
pub fn fechar_cais_aberto() {
    unsafe {
        let msg = RegisterWindowMessageW(MSG_SAIR);
        let _ = PostMessageW(HWND_BROADCAST, msg, WPARAM(0), LPARAM(0));
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, MUTEX) {
                Ok(h) => {
                    let _ = CloseHandle(h);
                }
                Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => {}
                Err(_) => return, // não existe mais: saiu
            }
        }
    }
}

// ----------------------------------------------------------------------
// Registro

unsafe fn gravar_texto(chave: HKEY, nome: PCWSTR, valor: &str) {
    let v: Vec<u16> = valor.encode_utf16().chain(Some(0)).collect();
    let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
    let _ = RegSetValueExW(chave, nome, 0, REG_SZ, Some(bytes));
}

unsafe fn gravar_numero(chave: HKEY, nome: PCWSTR, valor: u32) {
    let _ = RegSetValueExW(chave, nome, 0, REG_DWORD, Some(&valor.to_le_bytes()));
}

fn inicia_com_windows() -> bool {
    unsafe {
        let mut tam = 0u32;
        RegGetValueW(HKEY_CURRENT_USER, CHAVE_RUN, w!("Cais"), RRF_RT_REG_SZ, None, None, Some(&mut tam)).is_ok()
    }
}

fn definir_inicio(exe: Option<&Path>) {
    unsafe {
        let mut chave = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, CHAVE_RUN, 0, KEY_SET_VALUE, &mut chave).is_err() {
            return;
        }
        match exe {
            Some(e) => gravar_texto(chave, w!("Cais"), &format!("\"{}\"", e.display())),
            None => {
                let _ = RegDeleteValueW(chave, w!("Cais"));
            }
        }
        let _ = RegCloseKey(chave);
    }
}

// ----------------------------------------------------------------------
// Instalar

fn criar_atalho(exe: &Path, destino: &Path) -> windows::core::Result<()> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        if let Some(p) = exe.parent() {
            link.SetWorkingDirectory(&HSTRING::from(p.as_os_str()))?;
        }
        link.SetDescription(&HSTRING::from(crate::idioma::t("Cais: dock e barra de menu para o Windows", "Cais: dock and menu bar for Windows")))?;
        if let Some(p) = destino.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let arquivo: IPersistFile = link.cast()?;
        arquivo.Save(&HSTRING::from(destino.as_os_str()), true)?;
        Ok(())
    }
}

/// Copia o Cais para a pasta de programas e registra no Windows. Devolve o
/// caminho do cais.exe instalado.
pub fn instalar() -> Result<PathBuf, String> {
    let origem = crate::config::caminho_do_cais();
    let pasta = pasta_instalacao();
    let destino = exe_instalado();
    std::fs::create_dir_all(&pasta).map_err(|e| format!("{} {}: {e}", crate::idioma::t("Não foi possível criar a pasta", "Couldn't create the folder"), pasta.display()))?;
    if !mesmo_caminho(&origem, &destino) {
        // Atualizando um Cais instalado que está aberto: o Windows não deixa
        // sobrescrever, mas deixa renomear.
        if destino.exists() && std::fs::remove_file(&destino).is_err() {
            let velho = pasta.join(format!("cais-antigo-{}.exe", std::process::id()));
            let _ = std::fs::rename(&destino, velho);
        }
        std::fs::copy(&origem, &destino).map_err(|e| format!("{}: {e}", crate::idioma::t("Não foi possível copiar o Cais", "Couldn't copy Cais")))?;
    }
    for f in std::fs::read_dir(&pasta).into_iter().flatten().flatten() {
        let n = f.file_name().to_string_lossy().to_lowercase();
        if n.starts_with("cais-antigo-") {
            let _ = std::fs::remove_file(f.path()); // sobras de atualizações antigas
        }
    }
    let _ = criar_atalho(&destino, &atalho_do_menu());
    let tamanho_kb = std::fs::metadata(&destino).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
    unsafe {
        let mut chave = HKEY::default();
        if RegCreateKeyExW(HKEY_CURRENT_USER, CHAVE_DESINSTALAR, 0, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut chave, None).is_ok() {
            let exe = destino.display().to_string();
            gravar_texto(chave, w!("DisplayName"), "Cais");
            gravar_texto(chave, w!("DisplayVersion"), env!("CARGO_PKG_VERSION"));
            gravar_texto(chave, w!("Publisher"), crate::idioma::t("Cais (código aberto)", "Cais (open source)"));
            gravar_texto(chave, w!("InstallLocation"), &pasta.display().to_string());
            gravar_texto(chave, w!("DisplayIcon"), &format!("{exe},0"));
            gravar_texto(chave, w!("UninstallString"), &format!("\"{exe}\" --desinstalar"));
            gravar_numero(chave, w!("NoModify"), 1);
            gravar_numero(chave, w!("NoRepair"), 1);
            gravar_numero(chave, w!("EstimatedSize"), tamanho_kb);
            let _ = RegCloseKey(chave);
        }
    }
    // "Iniciar com o Windows" passa a apontar para o instalado.
    if inicia_com_windows() {
        definir_inicio(Some(&destino));
    }
    Ok(destino)
}

// ----------------------------------------------------------------------
// Desinstalar

fn perguntar(texto: &str, icone: MESSAGEBOX_STYLE) -> bool {
    unsafe { MessageBoxW(None, &HSTRING::from(texto), w!("Cais"), MB_YESNO | icone | MB_SETFOREGROUND) == IDYES }
}

fn avisar(texto: &str) {
    unsafe {
        MessageBoxW(None, &HSTRING::from(texto), w!("Cais"), MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND);
    }
}

/// Tira o Cais do Windows: atalho, inicialização, entrada em Aplicativos e
/// (se pedir) as configurações. A pasta do programa é apagada logo depois
/// que este processo fechar.
pub fn remover(apagar_config: bool) {
    crate::barra::mostrar();
    definir_inicio(None);
    let _ = std::fs::remove_file(atalho_do_menu());
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, CHAVE_DESINSTALAR);
    }
    if apagar_config {
        for var in ["APPDATA", "LOCALAPPDATA"] {
            if let Ok(base) = std::env::var(var) {
                let _ = std::fs::remove_dir_all(PathBuf::from(base).join("Cais"));
            }
        }
    }
    // O exe em uso não pode se apagar: um "cmd" espera 2 s e apaga a pasta.
    let pasta = pasta_instalacao();
    if pasta.exists() {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd.exe")
            .raw_arg(format!("/c ping 127.0.0.1 -n 3 >nul & rmdir /s /q \"{}\"", pasta.display()))
            .creation_flags(0x0800_0000) // sem janela
            .spawn();
    }
}

/// Perguntas e avisos da desinstalação. Devolve se foi confirmada (e a
/// escolha de apagar as configurações).
pub fn confirmar_desinstalar() -> Option<bool> {
    if !perguntar(
        crate::idioma::t(
            "Remover o Cais deste computador?\n\nA barra de tarefas do Windows volta ao normal.",
            "Remove Cais from this computer?\n\nThe Windows taskbar goes back to normal.",
        ),
        MB_ICONQUESTION,
    ) {
        return None;
    }
    let apagar = perguntar(
        crate::idioma::t(
            "Apagar também as suas configurações e os apps fixados no dock?\n\nEscolha \"Não\" se pretende instalar o Cais de novo.",
            "Also delete your settings and the apps pinned to the dock?\n\nChoose \"No\" if you plan to install Cais again.",
        ),
        MB_ICONQUESTION,
    );
    Some(apagar)
}

/// cais.exe --desinstalar (vem de Configurações > Aplicativos).
pub fn desinstalar_pela_linha_de_comando() {
    let Some(apagar) = confirmar_desinstalar() else { return };
    fechar_cais_aberto();
    remover(apagar);
    avisar(crate::idioma::t("O Cais foi removido.", "Cais was removed."));
}

/// cais.exe --instalar
pub fn instalar_pela_linha_de_comando() {
    match instalar() {
        Ok(exe) => {
            if perguntar(crate::idioma::t("O Cais foi instalado e está no menu Iniciar.\n\nAbrir agora?", "Cais is installed and in the Start menu.\n\nOpen it now?"), MB_ICONINFORMATION) {
                fechar_cais_aberto();
                let _ = std::process::Command::new(exe).spawn();
            }
        }
        Err(e) => avisar(&e),
    }
}
