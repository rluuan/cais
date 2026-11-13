//! Busca de arquivos pelo nome e pelo conteúdo, usando o índice do próprio
//! Windows (Windows Search, o mesmo do Explorador). Roda numa thread, um
//! instante depois que você para de digitar, e só dentro da sua pasta de
//! usuário. É tudo local: o índice fica no seu PC.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use windows::core::HSTRING;
use windows::Storage::Search::{FolderDepth, IndexerOption, QueryOptions};
use windows::Storage::StorageFolder;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Chegaram arquivos (wParam = geração da consulta).
pub const WM_ARQUIVOS: u32 = WM_APP + 34;
const ESPERA_MS: u64 = 220;

static GERACAO: AtomicU32 = AtomicU32::new(0);
static RESPOSTA: Mutex<Option<(u32, Vec<PathBuf>)>> = Mutex::new(None);

/// Procura `texto` e avisa `aviso` quando terminar. Devolve a geração (só a
/// resposta da última consulta interessa).
pub fn buscar(texto: String, max: u32, aviso: HWND) -> u32 {
    let g = GERACAO.fetch_add(1, Ordering::Relaxed) + 1;
    let h = aviso.0 as isize;
    std::thread::spawn(move || {
        // Espera parar de digitar.
        std::thread::sleep(std::time::Duration::from_millis(ESPERA_MS));
        if GERACAO.load(Ordering::Relaxed) != g {
            return;
        }
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let achados = consultar(&texto, max).unwrap_or_default();
        if GERACAO.load(Ordering::Relaxed) != g {
            return;
        }
        if let Ok(mut r) = RESPOSTA.lock() {
            *r = Some((g, achados));
        }
        unsafe {
            let _ = PostMessageW(HWND(h as *mut _), WM_ARQUIVOS, WPARAM(g as usize), LPARAM(0));
        }
    });
    g
}

/// Cancela a consulta em andamento (a resposta dela é ignorada).
pub fn cancelar() {
    GERACAO.fetch_add(1, Ordering::Relaxed);
}

/// A resposta da consulta `g`, se for a que chegou.
pub fn pegar(g: u32) -> Option<Vec<PathBuf>> {
    let mut r = RESPOSTA.lock().ok()?;
    match r.take() {
        Some((ge, v)) if ge == g => Some(v),
        outra => {
            *r = outra;
            None
        }
    }
}

fn consultar(texto: &str, max: u32) -> windows::core::Result<Vec<PathBuf>> {
    let raiz = std::env::var("USERPROFILE").unwrap_or_default();
    let pasta = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(raiz))?.get()?;
    let opcoes = QueryOptions::new()?;
    opcoes.SetFolderDepth(FolderDepth::Deep)?;
    // Só o índice: se o Windows não indexou, não sai varrendo o disco.
    opcoes.SetIndexerOption(IndexerOption::OnlyUseIndexer)?;
    opcoes.SetUserSearchFilter(&HSTRING::from(texto))?;
    let consulta = pasta.CreateFileQueryWithOptions(&opcoes)?;
    let arquivos = consulta.GetFilesAsync(0, max)?.get()?;
    let mut v = Vec::new();
    for f in arquivos {
        if let Ok(p) = f.Path() {
            v.push(PathBuf::from(p.to_string()));
        }
    }
    Ok(v)
}
