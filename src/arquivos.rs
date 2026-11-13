//! Base do Finder: listar pastas, ícones e miniaturas (numa thread), vigiar
//! a pasta aberta (o Windows avisa quando algo muda), buscar em subpastas e
//! o menu de contexto do próprio Windows (Abrir com, Copiar, Excluir...).
//! Tudo no seu disco; nada sai do PC.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::rc::Rc;
use windows::core::{implement, Interface, PCSTR, PCWSTR, PSTR};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub type Pixels = crate::apps::Pixels;

#[derive(Clone)]
pub struct Entrada {
    pub nome: String,
    pub caminho: PathBuf,
    pub pasta: bool,
    pub tamanho: u64,
    /// Data de modificação (FILETIME, 100 ns desde 1601).
    pub modificado: u64,
    pub tipo: String,
    /// Chave do ícone no cache (".txt" para tipos comuns, o caminho para
    /// pastas, programas, atalhos e miniaturas).
    pub chave: String,
    pub miniatura: bool,
}

const POR_ARQUIVO: &[&str] = &["exe", "lnk", "ico", "url", "msc", "appref-ms", "cpl", "scr"];
const IMAGENS: &[&str] = &["jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "tif", "tiff", "jfif"];
const VIDEOS: &[&str] = &["mp4", "mkv", "mov", "avi", "wmv", "webm", "m4v"];
pub const TEXTOS: &[&str] = &[
    "txt", "md", "log", "csv", "json", "xml", "ini", "cfg", "toml", "yaml", "yml", "rs", "py", "js", "ts", "html", "css",
    "c", "h", "cpp", "cs", "java", "sql", "ps1", "bat", "cmd", "sh",
];

pub fn extensao(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn eh_imagem(p: &Path) -> bool {
    IMAGENS.contains(&extensao(p).as_str())
}

pub fn eh_texto(p: &Path) -> bool {
    TEXTOS.contains(&extensao(p).as_str())
}

thread_local! {
    static TIPOS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// "Documento de Texto", "Imagem PNG"... (nome do tipo pelo Windows).
fn tipo_de(ext: &str, pasta: bool) -> String {
    if pasta {
        return crate::idioma::t("Pasta", "Folder").into();
    }
    TIPOS.with(|t| {
        if let Some(s) = t.borrow().get(ext) {
            return s.clone();
        }
        let nome: Vec<u16> = format!("a.{ext}").encode_utf16().chain(Some(0)).collect();
        let mut info = SHFILEINFOW::default();
        unsafe {
            SHGetFileInfoW(
                PCWSTR(nome.as_ptr()),
                FILE_ATTRIBUTE_NORMAL,
                Some(&mut info),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES,
            );
        }
        let n = info.szTypeName.iter().position(|&c| c == 0).unwrap_or(0);
        let s = String::from_utf16_lossy(&info.szTypeName[..n]);
        let s = if s.is_empty() { format!("{} {}", crate::idioma::t("Arquivo", "File"), ext.to_uppercase()) } else { s };
        t.borrow_mut().insert(ext.to_string(), s.clone());
        s
    })
}

/// Lugar "Este Computador" (os discos), que não é uma pasta de verdade.
pub const COMPUTADOR: &str = "::computador";
/// A Lixeira abre no Windows (não é uma pasta comum).
pub const LIXEIRA: &str = "::lixeira";

pub fn eh_computador(p: &Path) -> bool {
    p.as_os_str() == COMPUTADOR
}

/// Os discos como itens de "Este Computador".
fn listar_discos() -> Vec<Entrada> {
    discos()
        .into_iter()
        .filter(|(_, raiz)| !eh_computador(raiz))
        .map(|(nome, raiz)| {
            let w: Vec<u16> = raiz.to_string_lossy().encode_utf16().chain(Some(0)).collect();
            let tipo_disco = unsafe { GetDriveTypeW(PCWSTR(w.as_ptr())) };
            let (livre, total) = espaco_livre(&raiz).unwrap_or((0, 0));
            let tipo = match tipo_disco {
                2 => crate::idioma::t("Unidade removível", "Removable drive"),
                4 => crate::idioma::t("Unidade de rede", "Network drive"),
                5 => crate::idioma::t("Unidade de CD/DVD", "CD/DVD drive"),
                _ => crate::idioma::t("Disco local", "Local disk"),
            };
            let tipo = if total > 0 { format!("{tipo} ({} {} {})", tamanho_legivel(livre), crate::idioma::t("livres de", "free of"), tamanho_legivel(total)) } else { tipo.to_string() };
            Entrada {
                nome,
                caminho: raiz.clone(),
                pasta: true,
                tamanho: 0,
                modificado: 0,
                tipo,
                chave: raiz.to_string_lossy().into_owned(),
                miniatura: false,
            }
        })
        .collect()
}

/// Conteúdo da pasta (sem arquivos ocultos e de sistema), pastas primeiro.
pub fn listar(p: &Path) -> std::io::Result<Vec<Entrada>> {
    if eh_computador(p) {
        return Ok(listar_discos());
    }
    let mut v = Vec::new();
    for e in std::fs::read_dir(p)?.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let atrib = m.file_attributes();
        if atrib & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0 {
            continue;
        }
        v.push(entrada(e.path(), &m));
    }
    v.sort_by(|a, b| b.pasta.cmp(&a.pasta).then_with(|| a.nome.to_lowercase().cmp(&b.nome.to_lowercase())));
    Ok(v)
}

fn entrada(caminho: PathBuf, m: &std::fs::Metadata) -> Entrada {
    let pasta = m.is_dir();
    let ext = extensao(&caminho);
    let miniatura = !pasta && (IMAGENS.contains(&ext.as_str()) || VIDEOS.contains(&ext.as_str()));
    let por_arquivo = pasta || miniatura || POR_ARQUIVO.contains(&ext.as_str()) || ext.is_empty();
    let caminho_txt = caminho.to_string_lossy().into_owned();
    Entrada {
        nome: caminho.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(caminho_txt.clone()),
        tipo: tipo_de(&ext, pasta),
        chave: if por_arquivo { caminho_txt } else { format!(".{ext}") },
        caminho,
        pasta,
        tamanho: if pasta { 0 } else { m.len() },
        modificado: m.last_write_time(),
        miniatura,
    }
}

/// Lugares da barra lateral: (nome, caminho, é disco).
pub fn favoritos() -> Vec<(String, PathBuf)> {
    let pasta = |id: &windows::core::GUID| unsafe {
        SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok().map(|p| {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            PathBuf::from(s)
        })
    };
    let mut v = Vec::new();
    for (nome, id) in [
        (crate::idioma::t("Mesa", "Desktop"), FOLDERID_Desktop),
        (crate::idioma::t("Documentos", "Documents"), FOLDERID_Documents),
        ("Downloads", FOLDERID_Downloads),
        (crate::idioma::t("Imagens", "Pictures"), FOLDERID_Pictures),
        (crate::idioma::t("Músicas", "Music"), FOLDERID_Music),
        (crate::idioma::t("Vídeos", "Videos"), FOLDERID_Videos),
        (crate::idioma::t("Pasta pessoal", "Home"), FOLDERID_Profile),
    ] {
        if let Some(p) = pasta(&id).filter(|p| p.exists()) {
            v.push((nome.to_string(), p));
        }
    }
    v.push((crate::idioma::t("Lixeira", "Trash").to_string(), PathBuf::from(LIXEIRA)));
    v
}

/// Discos: (nome, raiz). O primeiro é "Este Computador".
pub fn discos() -> Vec<(String, PathBuf)> {
    let mascara = unsafe { GetLogicalDrives() };
    let mut v = vec![(crate::idioma::t("Este Computador", "This PC").to_string(), PathBuf::from(COMPUTADOR))];
    for i in 0..26u32 {
        if mascara & (1 << i) == 0 {
            continue;
        }
        let letra = (b'A' + i as u8) as char;
        let raiz = format!("{letra}:\\");
        let w: Vec<u16> = raiz.encode_utf16().chain(Some(0)).collect();
        let tipo = unsafe { GetDriveTypeW(PCWSTR(w.as_ptr())) };
        if tipo == 1 || tipo == 0 {
            continue; // não existe
        }
        let mut nome = [0u16; 64];
        let ok = unsafe { GetVolumeInformationW(PCWSTR(w.as_ptr()), Some(&mut nome), None, None, None, None) }.is_ok();
        let n = nome.iter().position(|&c| c == 0).unwrap_or(0);
        let rotulo = if ok && n > 0 { String::from_utf16_lossy(&nome[..n]) } else { crate::idioma::t("Disco Local", "Local Disk").into() };
        v.push((format!("{rotulo} ({letra}:)"), PathBuf::from(raiz)));
    }
    v
}

/// (livre, total) em bytes do disco da pasta.
pub fn espaco_livre(p: &Path) -> Option<(u64, u64)> {
    let w: Vec<u16> = p.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let (mut livre, mut total) = (0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(PCWSTR(w.as_ptr()), Some(&mut livre), Some(&mut total), None).ok()? };
    Some((livre, total))
}

pub fn tamanho_legivel(b: u64) -> String {
    let f = b as f64;
    let (v, u) = if f >= 1024.0 * 1024.0 * 1024.0 {
        (f / 1024.0 / 1024.0 / 1024.0, "GB")
    } else if f >= 1024.0 * 1024.0 {
        (f / 1024.0 / 1024.0, "MB")
    } else if f >= 1024.0 {
        (f / 1024.0, "KB")
    } else {
        return format!("{b} bytes");
    };
    format!("{:.1} {u}", v).replace('.', ",")
}

/// FILETIME -> "12/11/2026 14:05" (hora local).
pub fn data_legivel(ft: u64) -> String {
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let f = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let (mut utc, mut local) = (Default::default(), Default::default());
    unsafe {
        if FileTimeToSystemTime(&f, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            return String::new();
        }
    }
    let l: windows::Win32::Foundation::SYSTEMTIME = local;
    format!("{:02}/{:02}/{} {:02}:{:02}", l.wDay, l.wMonth, l.wYear, l.wHour, l.wMinute)
}

// ---------------------------------------------------------------------------
// Ícones e miniaturas numa thread (o Windows pode demorar em arquivos grandes)

struct Pedido {
    chave: String,
    caminho: String,
    px: u32,
    miniatura: bool,
}

pub struct ServicoIcones {
    tx: Sender<Pedido>,
    prontos: Arc<Mutex<Vec<(String, Option<Pixels>)>>>,
    pendentes: HashSet<String>,
}

impl ServicoIcones {
    pub fn new(hwnd_aviso: HWND, msg: u32) -> Self {
        let (tx, rx) = channel::<Pedido>();
        let prontos: Arc<Mutex<Vec<(String, Option<Pixels>)>>> = Arc::new(Mutex::new(Vec::new()));
        let p = prontos.clone();
        let aviso = hwnd_aviso.0 as isize;
        let _ = std::thread::Builder::new()
            .name("cais-icones".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                // MTA: a thread não atende mensagens, então não pode ser STA
                // (o Shell travaria esperando por ela).
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                while let Ok(primeiro) = rx.recv() {
                    // Os mais recentes primeiro (o que está na tela agora).
                    let mut fila = vec![primeiro];
                    while let Ok(x) = rx.try_recv() {
                        fila.push(x);
                    }
                    for (k, pd) in fila.into_iter().rev().enumerate() {
                        let flags = if pd.miniatura { SIIGBF_THUMBNAILONLY } else { SIIGBF_ICONONLY };
                        let mut r = imagem_do_shell(&pd.caminho, pd.px, flags);
                        if r.is_none() && pd.miniatura {
                            r = imagem_do_shell(&pd.caminho, pd.px, SIIGBF_ICONONLY);
                        }
                        if let Ok(mut v) = p.lock() {
                            v.push((pd.chave, r));
                        }
                        if k % 12 == 11 {
                            unsafe {
                                let _ = PostMessageW(HWND(aviso as *mut _), msg, WPARAM(0), LPARAM(0));
                            }
                        }
                    }
                    unsafe {
                        let _ = PostMessageW(HWND(aviso as *mut _), msg, WPARAM(0), LPARAM(0));
                    }
                }
            });
        Self { tx, prontos, pendentes: HashSet::new() }
    }

    /// Pede um ícone (se ainda não foi pedido).
    pub fn pedir(&mut self, chave: &str, caminho: &Path, px: u32, miniatura: bool) {
        if !self.pendentes.insert(chave.to_string()) {
            return;
        }
        let _ = self.tx.send(Pedido {
            chave: chave.to_string(),
            caminho: caminho.to_string_lossy().into_owned(),
            px,
            miniatura,
        });
    }

    pub fn receber(&mut self) -> Vec<(String, Option<Pixels>)> {
        let v = self.prontos.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
        for (c, _) in &v {
            self.pendentes.remove(c);
        }
        v
    }

    /// Esquece o que foi pedido (ex.: ao trocar de tamanho dos ícones).
    pub fn esquecer(&mut self, chave: &str) {
        self.pendentes.remove(chave);
    }
}

/// Imagem do Windows para um arquivo no tamanho `px` (BGRA pré-multiplicado).
pub fn imagem_do_shell(caminho: &str, px: u32, flags: SIIGBF) -> Option<Pixels> {
    unsafe {
        let w: Vec<u16> = caminho.encode_utf16().chain(Some(0)).collect();
        let fabrica: IShellItemImageFactory = SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None).ok()?;
        let hbm = fabrica.GetImage(SIZE { cx: px as i32, cy: px as i32 }, flags | SIIGBF_BIGGERSIZEOK).ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(HGDIOBJ(hbm.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _));
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
        let mut pixels = vec![0u8; (l * a * 4).max(0) as usize];
        let hdc = GetDC(None);
        let linhas = GetDIBits(hdc, hbm, 0, a as u32, Some(pixels.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
        ReleaseDC(None, hdc);
        let _ = DeleteObject(HGDIOBJ(hbm.0));
        if linhas == 0 || l == 0 {
            return None;
        }
        if pixels.chunks(4).all(|p| p[3] == 0) {
            for p in pixels.chunks_mut(4) {
                p[3] = 255;
            }
        }
        Some((l as u32, a as u32, pixels))
    }
}

// ---------------------------------------------------------------------------
// Vigia: o Windows avisa quando algo muda na pasta aberta

pub struct Vigia {
    parar: HANDLE,
}

impl Drop for Vigia {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.parar);
        }
    }
}

pub fn vigiar(pasta: &Path, hwnd: HWND, msg: u32) -> Option<Vigia> {
    unsafe {
        let parar = CreateEventW(None, true, false, PCWSTR::null()).ok()?;
        let w: Vec<u16> = pasta.to_string_lossy().encode_utf16().chain(Some(0)).collect();
        let filtro = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE;
        let mudou = FindFirstChangeNotificationW(PCWSTR(w.as_ptr()), false, filtro).ok()?;
        let (p, m, h) = (parar.0 as isize, mudou.0 as isize, hwnd.0 as isize);
        let _ = std::thread::Builder::new().name("cais-vigia".into()).stack_size(64 * 1024).spawn(move || {
            let alcas = [HANDLE(p as *mut _), HANDLE(m as *mut _)];
            loop {
                let r = WaitForMultipleObjects(&alcas, false, INFINITE);
                if r.0 != WAIT_OBJECT_0.0 + 1 {
                    break; // pediram para parar (ou deu erro)
                }
                let _ = PostMessageW(HWND(h as *mut _), msg, WPARAM(0), LPARAM(0));
                if FindNextChangeNotification(alcas[1]).is_err() {
                    break;
                }
            }
            let _ = FindCloseChangeNotification(alcas[1]);
            let _ = CloseHandle(alcas[0]);
        });
        Some(Vigia { parar })
    }
}

// ---------------------------------------------------------------------------
// Busca em subpastas (numa thread, com limite e cancelável)

const PULAR: &[&str] = &["node_modules", ".git", "appdata", "$recycle.bin", "windows", "target", "system volume information"];

pub fn buscar(raiz: PathBuf, termo: String, hwnd: HWND, msg: u32, cancelar: Arc<AtomicBool>, saida: Arc<Mutex<Vec<Entrada>>>) {
    let h = hwnd.0 as isize;
    let _ = std::thread::Builder::new().name("cais-busca".into()).stack_size(512 * 1024).spawn(move || {
        let termo = crate::catalogo::normalizar(&termo);
        let mut fila = vec![raiz];
        let mut achados = 0usize;
        let mut lote = Vec::new();
        let avisar = |lote: &mut Vec<Entrada>| {
            if let Ok(mut s) = saida.lock() {
                s.append(lote);
            }
            unsafe {
                let _ = PostMessageW(HWND(h as *mut _), msg, WPARAM(0), LPARAM(0));
            }
        };
        while let Some(p) = fila.pop() {
            if cancelar.load(Ordering::Relaxed) || achados >= 2000 {
                break;
            }
            let Ok(rd) = std::fs::read_dir(&p) else { continue };
            for e in rd.flatten() {
                let Ok(m) = e.metadata() else { continue };
                if m.file_attributes() & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0 {
                    continue;
                }
                let nome = e.file_name().to_string_lossy().to_string();
                if crate::catalogo::normalizar(&nome).contains(&termo) {
                    lote.push(entrada(e.path(), &m));
                    achados += 1;
                }
                if m.is_dir() && !PULAR.contains(&nome.to_lowercase().as_str()) {
                    fila.insert(0, e.path()); // largura primeiro: perto do início antes
                }
            }
            if lote.len() >= 40 {
                avisar(&mut lote);
            }
        }
        avisar(&mut lote);
    });
}

// ---------------------------------------------------------------------------
// Menu de contexto do Windows e verbos (copiar, colar, excluir...)

thread_local! {
    /// Menu do Windows aberto agora (os submenus "Abrir com" e "Enviar
    /// para" se montam por mensagens que repassamos a ele).
    pub static MENU_ATIVO: RefCell<Option<IContextMenu2>> = const { RefCell::new(None) };
}

/// Repassa ao menu do Windows aberto as mensagens dos submenus dele.
pub fn mensagem_do_menu(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    if !matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        return None;
    }
    MENU_ATIVO.with(|m| {
        let m = m.try_borrow().ok()?;
        let cm = m.as_ref()?;
        unsafe {
            if let Ok(c3) = cm.cast::<IContextMenu3>() {
                let mut r = LRESULT(0);
                c3.HandleMenuMsg2(msg, wp, lp, Some(&mut r)).ok()?;
                return Some(r);
            }
            cm.HandleMenuMsg(msg, wp, lp).ok()?;
            Some(LRESULT(0))
        }
    })
}

struct Pidl(*mut ITEMIDLIST);
impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0 as _)) };
    }
}

fn pidl(p: &Path) -> Option<Pidl> {
    let w: Vec<u16> = p.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let mut r = std::ptr::null_mut();
    unsafe { SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut r, 0, None).ok()? };
    (!r.is_null()).then_some(Pidl(r))
}

/// Menu de contexto do Windows para os arquivos (todos da mesma pasta).
fn menu_dos_itens(hwnd: HWND, itens: &[PathBuf]) -> Option<IContextMenu> {
    let pidls: Vec<Pidl> = itens.iter().filter_map(|p| pidl(p)).collect();
    if pidls.is_empty() {
        return None;
    }
    unsafe {
        let pai: IShellFolder = SHBindToParent(pidls[0].0, None).ok()?;
        let filhos: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| ILFindLastID(p.0) as *const _).collect();
        pai.GetUIObjectOf(hwnd, &filhos, None).ok()
    }
}

/// Menu do fundo da pasta (Novo, Colar, Propriedades...).
fn menu_da_pasta(hwnd: HWND, pasta: &Path) -> Option<IContextMenu> {
    let p = pidl(pasta)?;
    unsafe {
        let mesa = SHGetDesktopFolder().ok()?;
        let f: IShellFolder = if (*p.0).mkid.cb == 0 { mesa } else { mesa.BindToObject(p.0, None).ok()? };
        f.CreateViewObject(hwnd).ok()
    }
}

pub enum Escolha {
    Nada,
    Renomear,
    /// Itens do Cais no topo do menu (uma pasta só).
    NovaAba,
    NoExplorador,
    Feito,
}

const ID_NOVA_ABA: u32 = 0x8001;
const ID_EXPLORADOR: u32 = 0x8002;

/// Mostra o menu do Windows em (x, y) da tela e executa o que você
/// escolher. "Renomear" volta para o Finder fazer na própria janela.
pub fn menu_contexto(hwnd: HWND, itens: &[PathBuf], pasta: &Path, x: i32, y: i32) -> Escolha {
    let cm = if itens.is_empty() { menu_da_pasta(hwnd, pasta) } else { menu_dos_itens(hwnd, itens) };
    let Some(cm) = cm else { return Escolha::Nada };
    unsafe {
        let Ok(m) = CreatePopupMenu() else { return Escolha::Nada };
        let mut flags = CMF_NORMAL | CMF_CANRENAME;
        if GetKeyState(0x10) < 0 {
            flags |= CMF_EXTENDEDVERBS; // Shift: verbos extras, como no Explorer
        }
        if cm.QueryContextMenu(m, 0, 1, 0x7FFF, flags).is_err() {
            let _ = DestroyMenu(m);
            return Escolha::Nada;
        }
        // Uma pasta só: "Abrir em nova aba" e "Mostrar no Explorador" no topo.
        if itens.len() == 1 && itens[0].is_dir() {
            let _ = InsertMenuW(m, 0, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null());
            let _ = InsertMenuW(m, 0, MF_BYPOSITION | MF_STRING, ID_EXPLORADOR as usize, PCWSTR(crate::idioma::largo(crate::idioma::t("Mostrar no Explorador do Windows", "Show in File Explorer")).as_ptr()));
            let _ = InsertMenuW(m, 0, MF_BYPOSITION | MF_STRING, ID_NOVA_ABA as usize, PCWSTR(crate::idioma::largo(crate::idioma::t("Abrir em nova aba", "Open in New Tab")).as_ptr()));
        }
        MENU_ATIVO.with(|a| *a.borrow_mut() = cm.cast::<IContextMenu2>().ok());
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenuEx(m, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, x, y, hwnd, None).0 as u32;
        MENU_ATIVO.with(|a| *a.borrow_mut() = None);
        let _ = DestroyMenu(m);
        if cmd == 0 {
            return Escolha::Nada;
        }
        if cmd == ID_NOVA_ABA {
            return Escolha::NovaAba;
        }
        if cmd == ID_EXPLORADOR {
            return Escolha::NoExplorador;
        }
        // O verbo "rename" precisa de uma visão do Explorer: fazemos nós.
        let mut verbo = [0u8; 64];
        if cm
            .GetCommandString((cmd - 1) as usize, GCS_VERBA, None, PSTR(verbo.as_mut_ptr()), verbo.len() as u32)
            .is_ok()
        {
            let n = verbo.iter().position(|&c| c == 0).unwrap_or(0);
            if &verbo[..n] == b"rename" {
                return Escolha::Renomear;
            }
        }
        invocar(&cm, hwnd, PCSTR((cmd - 1) as usize as *const u8), x, y);
        Escolha::Feito
    }
}

unsafe fn invocar(cm: &IContextMenu, hwnd: HWND, verbo: PCSTR, x: i32, y: i32) {
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: CMIC_MASK_PTINVOKE,
        hwnd,
        lpVerb: verbo,
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: POINT { x, y },
        ..Default::default()
    };
    let _ = cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO);
}

/// Executa um verbo do Windows ("copy", "cut", "paste", "delete",
/// "properties") nos itens (ou na pasta, se não houver itens).
pub fn verbo(hwnd: HWND, itens: &[PathBuf], pasta: &Path, nome: &str) {
    let cm = if itens.is_empty() { menu_da_pasta(hwnd, pasta) } else { menu_dos_itens(hwnd, itens) };
    let Some(cm) = cm else { return };
    unsafe {
        let Ok(m) = CreatePopupMenu() else { return };
        let _ = cm.QueryContextMenu(m, 0, 1, 0x7FFF, CMF_NORMAL);
        let v: Vec<u8> = nome.bytes().chain(Some(0)).collect();
        invocar(&cm, hwnd, PCSTR(v.as_ptr()), 0, 0);
        let _ = DestroyMenu(m);
    }
}

// ----------------------------------------------------------------------
// Arrastar e soltar

thread_local! {
    /// Pasta de onde estamos arrastando agora (para não "soltar em si mesmo").
    static ORIGEM: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Arrasta os itens para fora do Finder: área de trabalho, Explorer, e-mail,
/// outro Finder... O Windows cuida da imagem que segue o mouse e de copiar
/// ou mover. Bloqueia até você soltar.
pub fn arrastar(hwnd: HWND, itens: &[PathBuf], pasta: &Path) {
    let pidls: Vec<Pidl> = itens.iter().filter_map(|p| pidl(p)).collect();
    if pidls.is_empty() {
        return;
    }
    unsafe {
        let Ok(pai) = SHBindToParent::<IShellFolder>(pidls[0].0, None) else { return };
        let filhos: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| ILFindLastID(p.0) as *const _).collect();
        let Ok(dados) = pai.GetUIObjectOf::<_, IDataObject>(hwnd, &filhos, None) else { return };
        ORIGEM.with(|o| *o.borrow_mut() = Some(pasta.to_path_buf()));
        let efeitos = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
        let _ = SHDoDragDrop(hwnd, &dados, None, efeitos);
        ORIGEM.with(|o| *o.borrow_mut() = None);
    }
}

/// O que o Finder conta para quem recebe arquivos soltos: a pasta aberta e
/// onde estão as pastas na tela (em pixels da janela). Quem recebe escreve
/// em `destaque` a pasta sob o mouse, para o Finder pintar.
#[derive(Default)]
pub struct MapaSoltar {
    pub pasta: PathBuf,
    pub pastas: Vec<(RECT, PathBuf)>,
    pub destaque: Option<PathBuf>,
}

/// Recebe arquivos soltos na janela do Finder e repassa para a própria
/// pasta do Windows (a mesma coisa que soltar no Explorer: copia, move,
/// pergunta se o arquivo já existe, mostra o progresso).
#[implement(IDropTarget)]
pub struct Soltar {
    hwnd: HWND,
    aviso: u32,
    mapa: Rc<RefCell<MapaSoltar>>,
    dados: RefCell<Option<IDataObject>>,
    alvo: RefCell<Option<(PathBuf, Option<IDropTarget>)>>,
    ajudante: Option<IDropTargetHelper>,
}

impl Soltar {
    pub fn registrar(hwnd: HWND, aviso: u32, mapa: Rc<RefCell<MapaSoltar>>) {
        let ajudante = unsafe { CoCreateInstance::<_, IDropTargetHelper>(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok() };
        let s: IDropTarget = Soltar { hwnd, aviso, mapa, dados: RefCell::new(None), alvo: RefCell::new(None), ajudante }.into();
        unsafe {
            let _ = RegisterDragDrop(hwnd, &s);
        }
    }

    pub fn revogar(hwnd: HWND) {
        unsafe {
            let _ = RevokeDragDrop(hwnd);
        }
    }

    /// Pasta sob o ponto (tela): uma pasta da lista ou a pasta aberta.
    fn destino(&self, pt: &POINTL) -> PathBuf {
        let mut p = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut p);
        }
        let m = self.mapa.borrow();
        m.pastas
            .iter()
            .find(|(r, _)| p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom)
            .map(|(_, c)| c.clone())
            .unwrap_or_else(|| m.pasta.clone())
    }

    /// Troca o alvo quando o mouse passa para outra pasta e repassa o
    /// "entrou" para a pasta nova.
    fn mirar(&self, pt: &POINTL, teclas: MODIFIERKEYS_FLAGS, efeito: *mut DROPEFFECT) {
        let destino = self.destino(pt);
        let igual = self.alvo.borrow().as_ref().is_some_and(|(p, _)| *p == destino);
        if !igual {
            if let Some((_, Some(a))) = self.alvo.borrow_mut().take() {
                unsafe {
                    let _ = a.DragLeave();
                }
            }
            // Soltar na mesma pasta de onde saiu não faz nada.
            let mesma = ORIGEM.with(|o| o.borrow().as_ref() == Some(&destino));
            let novo = if mesma { None } else { alvo_da_pasta(self.hwnd, &destino) };
            if let (Some(a), Some(d)) = (&novo, self.dados.borrow().as_ref()) {
                unsafe {
                    let _ = a.DragEnter(d, teclas, *pt, efeito);
                }
            }
            {
                let mut m = self.mapa.borrow_mut();
                m.destaque = (destino != m.pasta && novo.is_some()).then(|| destino.clone());
            }
            *self.alvo.borrow_mut() = Some((destino, novo));
            unsafe {
                let _ = PostMessageW(self.hwnd, self.aviso, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn esquecer(&self) {
        if let Some((_, Some(a))) = self.alvo.borrow_mut().take() {
            unsafe {
                let _ = a.DragLeave();
            }
        }
        *self.dados.borrow_mut() = None;
        self.mapa.borrow_mut().destaque = None;
        unsafe {
            let _ = PostMessageW(self.hwnd, self.aviso, WPARAM(0), LPARAM(0));
        }
    }
}

/// O "receptor" que o próprio Windows usa para a pasta.
fn alvo_da_pasta(hwnd: HWND, pasta: &Path) -> Option<IDropTarget> {
    let p = pidl(pasta)?;
    unsafe {
        let pai: IShellFolder = SHBindToParent(p.0, None).ok()?;
        let filho = [ILFindLastID(p.0) as *const ITEMIDLIST];
        pai.GetUIObjectOf(hwnd, &filho, None).ok()
    }
}

impl IDropTarget_Impl for Soltar_Impl {
    fn DragEnter(&self, dados: Option<&IDataObject>, teclas: MODIFIERKEYS_FLAGS, pt: &POINTL, efeito: *mut DROPEFFECT) -> windows::core::Result<()> {
        *self.dados.borrow_mut() = dados.cloned();
        *self.alvo.borrow_mut() = None;
        let pedido = unsafe { *efeito };
        self.mirar(pt, teclas, efeito);
        let pronto = self.alvo.borrow().as_ref().is_some_and(|(_, a)| a.is_some());
        unsafe {
            if !pronto {
                *efeito = DROPEFFECT_NONE;
            }
            if let (Some(h), Some(d)) = (&self.ajudante, dados) {
                let _ = h.DragEnter(self.hwnd, d, &POINT { x: pt.x, y: pt.y }, *efeito);
            }
        }
        let _ = pedido;
        Ok(())
    }

    fn DragOver(&self, teclas: MODIFIERKEYS_FLAGS, pt: &POINTL, efeito: *mut DROPEFFECT) -> windows::core::Result<()> {
        self.mirar(pt, teclas, efeito);
        unsafe {
            match self.alvo.borrow().as_ref() {
                Some((_, Some(a))) => {
                    if a.DragOver(teclas, *pt, efeito).is_err() {
                        *efeito = DROPEFFECT_NONE;
                    }
                }
                _ => *efeito = DROPEFFECT_NONE,
            }
            if let Some(h) = &self.ajudante {
                let _ = h.DragOver(&POINT { x: pt.x, y: pt.y }, *efeito);
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if let Some(h) = &self.ajudante {
            unsafe {
                let _ = h.DragLeave();
            }
        }
        self.esquecer();
        Ok(())
    }

    fn Drop(&self, dados: Option<&IDataObject>, teclas: MODIFIERKEYS_FLAGS, pt: &POINTL, efeito: *mut DROPEFFECT) -> windows::core::Result<()> {
        unsafe {
            if let (Some(h), Some(d)) = (&self.ajudante, dados) {
                let _ = h.Drop(d, &POINT { x: pt.x, y: pt.y }, *efeito);
            }
            let alvo = self.alvo.borrow_mut().take();
            match (alvo, dados) {
                (Some((_, Some(a))), Some(d)) => {
                    if a.Drop(d, teclas, *pt, efeito).is_err() {
                        *efeito = DROPEFFECT_NONE;
                    }
                }
                _ => *efeito = DROPEFFECT_NONE,
            }
        }
        *self.dados.borrow_mut() = None;
        self.mapa.borrow_mut().destaque = None;
        unsafe {
            let _ = PostMessageW(self.hwnd, self.aviso, WPARAM(0), LPARAM(0));
        }
        Ok(())
    }
}

/// Abre o arquivo com o programa padrão (pasta: o Finder trata).
pub fn abrir(p: &Path) {
    crate::apps::abrir(&p.to_string_lossy());
}

/// Abre a pasta no Explorador do Windows (pedido seu: o Finder não troca).
pub fn abrir_no_explorador(p: &Path) {
    let _ = std::process::Command::new(r"C:\Windows\explorer.exe").arg(p).spawn();
}

/// Abre a Lixeira no Windows (o Finder não lista a Lixeira).
pub fn abrir_lixeira() {
    crate::pastas::deixar_passar();
    crate::apps::abrir("shell:RecycleBinFolder");
}

/// A Lixeira tem algum item? (para o ícone do dock aparecer cheio)
pub fn lixeira_tem_algo() -> bool {
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info).is_ok() && info.i64NumItems > 0 }
}

/// Esvazia a Lixeira. O próprio Windows pergunta antes (não dá para desfazer).
pub fn esvaziar_lixeira(dono: HWND) {
    unsafe {
        let _ = SHEmptyRecycleBinW(dono, PCWSTR::null(), 0);
    }
}

pub fn eh_zip(p: &Path) -> bool {
    extensao(p) == "zip"
}

/// Como o Utilitário de Arquivo do Mac: extrai o .zip numa pasta ao lado
/// (com o mesmo nome; se já existir, "nome 2"...) usando o tar do próprio
/// Windows, numa thread. Avisa `hwnd` com a pasta criada (wParam = ok).
pub fn extrair_zip(zip: PathBuf, hwnd: HWND, msg: u32, saida: Arc<Mutex<Option<PathBuf>>>) {
    let Some(dir) = zip.parent().map(|p| p.to_path_buf()) else { return };
    let base = zip.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Arquivo".into());
    let mut destino = dir.join(&base);
    let mut k = 2;
    while destino.exists() {
        destino = dir.join(format!("{base} {k}"));
        k += 1;
    }
    let h = hwnd.0 as isize;
    std::thread::spawn(move || {
        let ok = std::fs::create_dir(&destino).is_ok()
            && std::process::Command::new(r"C:\Windows\System32\tar.exe")
                .arg("-xf")
                .arg(&zip)
                .arg("-C")
                .arg(&destino)
                .creation_flags_sem_janela()
                .status()
                .is_ok_and(|s| s.success());
        if !ok {
            let _ = std::fs::remove_dir(&destino); // só some se ficou vazia
        }
        if let Ok(mut g) = saida.lock() {
            *g = ok.then_some(destino);
        }
        unsafe {
            let _ = PostMessageW(HWND(h as *mut _), msg, WPARAM(ok as usize), LPARAM(0));
        }
    });
}

trait SemJanela {
    fn creation_flags_sem_janela(&mut self) -> &mut Self;
}

impl SemJanela for std::process::Command {
    fn creation_flags_sem_janela(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        self.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
    }
}

pub fn nova_pasta(dentro: &Path) -> Option<PathBuf> {
    let mut nome = dentro.join(crate::idioma::t("Nova pasta", "New Folder"));
    let mut k = 2;
    while nome.exists() {
        nome = dentro.join(format!("{} ({k})", crate::idioma::t("Nova pasta", "New Folder")));
        k += 1;
    }
    std::fs::create_dir(&nome).ok()?;
    Some(nome)
}

