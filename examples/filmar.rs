//! Ferramenta de teste (não faz parte do Cais): filma um pedaço da tela
//! quadro a quadro (Desktop Duplication) e, opcionalmente, clica num ponto
//! no meio da filmagem. Grava BMPs reduzidos e um CSV com o brilho médio e
//! a diferença entre quadros, para achar piscadas. Tudo local.
//!
//! cargo run --release --example filmar -- --regiao l t r b --ms 1500
//!     [--clicar x y] [--antes 200] [--reduzir 4] [--saida pasta] [--sem-fotos]

use std::io::Write;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

struct Quadro {
    t: f64,
    w: usize,
    h: usize,
    px: Vec<u8>, // BGR
}

fn clicar(x: i32, y: i32) {
    unsafe {
        let _ = SetCursorPos(x - 3, y);
        let mk = |f: MOUSE_EVENT_FLAGS, dx: i32| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy: 0, mouseData: 0, dwFlags: f, time: 0, dwExtraInfo: 0 } },
        };
        let n0 = SendInput(&[mk(MOUSEEVENTF_MOVE, 3)], std::mem::size_of::<INPUT>() as i32);
        std::thread::sleep(Duration::from_millis(120));
        let n1 = SendInput(&[mk(MOUSEEVENTF_LEFTDOWN, 0)], std::mem::size_of::<INPUT>() as i32);
        std::thread::sleep(Duration::from_millis(50));
        let n2 = SendInput(&[mk(MOUSEEVENTF_LEFTUP, 0)], std::mem::size_of::<INPUT>() as i32);
        eprintln!("SendInput: {n0} {n1} {n2} (erro {:?})", GetLastError());
    }
}

fn salvar_bmp(caminho: &std::path::Path, q: &Quadro) {
    let linha = (q.w * 3 + 3) & !3;
    let tam = 54 + linha * q.h;
    let mut b = Vec::with_capacity(tam);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(tam as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(q.w as i32).to_le_bytes());
    b.extend_from_slice(&(-(q.h as i32)).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0u8; 24]);
    for y in 0..q.h {
        b.extend_from_slice(&q.px[y * q.w * 3..(y + 1) * q.w * 3]);
        b.extend(std::iter::repeat(0u8).take(linha - q.w * 3));
    }
    let _ = std::fs::write(caminho, b);
}

fn main() -> windows::core::Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let a: Vec<String> = std::env::args().collect();
    let pegar = |nome: &str, n: usize| -> Option<Vec<i32>> {
        let i = a.iter().position(|s| s == nome)?;
        Some(a[i + 1..i + 1 + n].iter().map(|s| s.parse().unwrap()).collect())
    };
    let regiao = pegar("--regiao", 4).expect("--regiao l t r b");
    let ms = pegar("--ms", 1).map_or(1500, |v| v[0]) as u64;
    let antes = pegar("--antes", 1).map_or(200, |v| v[0]) as u64;
    let red = pegar("--reduzir", 1).map_or(4, |v| v[0]).max(1) as usize;
    let clique = pegar("--clicar", 2);
    let sem_fotos = a.iter().any(|s| s == "--sem-fotos");
    let saida = a.iter().position(|s| s == "--saida").map(|i| a[i + 1].clone()).unwrap_or("filme".into());
    let saida = std::path::PathBuf::from(saida);
    let _ = std::fs::remove_dir_all(&saida);
    std::fs::create_dir_all(&saida).unwrap();

    unsafe {
        let mut dev = None;
        let mut ctx = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut dev),
            None,
            Some(&mut ctx),
        )?;
        let dev: ID3D11Device = dev.unwrap();
        let ctx: ID3D11DeviceContext = ctx.unwrap();
        let adaptador = dev.cast::<IDXGIDevice>()?.GetAdapter()?;
        // Saída (monitor) que contém o canto da região.
        let mut i = 0;
        let (saida_dxgi, origem) = loop {
            let o = adaptador.EnumOutputs(i)?;
            let d = o.GetDesc()?;
            let r = d.DesktopCoordinates;
            if regiao[0] >= r.left && regiao[0] < r.right && regiao[1] >= r.top && regiao[1] < r.bottom {
                break (o, (r.left, r.top));
            }
            i += 1;
        };
        let dup = saida_dxgi.cast::<IDXGIOutput1>()?.DuplicateOutput(&dev)?;
        let (l, t) = ((regiao[0] - origem.0) as usize, (regiao[1] - origem.1) as usize);
        let (rw, rh) = ((regiao[2] - regiao[0]) as usize, (regiao[3] - regiao[1]) as usize);
        let (w, h) = (rw / red, rh / red);
        let mut quadros: Vec<Quadro> = Vec::new();
        let mut staging: Option<ID3D11Texture2D> = None;
        let inicio = Instant::now();
        let mut t_clique: Option<f64> = None;
        let mut ultimo: Option<Vec<u8>> = None;
        while inicio.elapsed() < Duration::from_millis(antes + ms) {
            if clique.is_some() && t_clique.is_none() && inicio.elapsed() >= Duration::from_millis(antes) {
                let c = clique.as_ref().unwrap();
                clicar(c[0], c[1]);
                t_clique = Some(inicio.elapsed().as_secs_f64() * 1000.0);
            }
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut rec = None;
            match dup.AcquireNextFrame(20, &mut info, &mut rec) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
                Err(e) => return Err(e),
            }
            let agora = inicio.elapsed().as_secs_f64() * 1000.0;
            if info.LastPresentTime == 0 {
                let _ = dup.ReleaseFrame();
                continue; // só o mouse mexeu
            }
            let tex: ID3D11Texture2D = rec.unwrap().cast()?;
            if staging.is_none() {
                let mut d = D3D11_TEXTURE2D_DESC::default();
                tex.GetDesc(&mut d);
                d.Usage = D3D11_USAGE_STAGING;
                d.BindFlags = 0;
                d.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
                d.MiscFlags = 0;
                let mut s = None;
                dev.CreateTexture2D(&d, None, Some(&mut s))?;
                staging = s;
            }
            let st = staging.as_ref().unwrap();
            ctx.CopyResource(st, &tex);
            let _ = dup.ReleaseFrame();
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(st, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
            let base = m.pData as *const u8;
            let mut px = vec![0u8; w * h * 3];
            for y in 0..h {
                let linha = base.add((t + y * red) * m.RowPitch as usize);
                for x in 0..w {
                    let p = linha.add((l + x * red) * 4);
                    let o = (y * w + x) * 3;
                    px[o] = *p;
                    px[o + 1] = *p.add(1);
                    px[o + 2] = *p.add(2);
                }
            }
            ctx.Unmap(st, 0);
            if ultimo.as_ref() == Some(&px) {
                continue; // a região não mudou
            }
            ultimo = Some(px.clone());
            quadros.push(Quadro { t: agora, w, h, px });
        }
        let t0 = t_clique.unwrap_or(0.0);
        let mut csv = std::fs::File::create(saida.join("quadros.csv")).unwrap();
        let _ = writeln!(csv, "n;ms_desde_clique;brilho;dif_anterior");
        let mut ant: Option<&Vec<u8>> = None;
        for (n, q) in quadros.iter().enumerate() {
            let brilho = q.px.iter().map(|&v| v as u64).sum::<u64>() as f64 / q.px.len() as f64;
            let dif = ant.map_or(0.0, |p| {
                p.iter().zip(&q.px).map(|(&a, &b)| (a as i32 - b as i32).unsigned_abs() as u64).sum::<u64>() as f64 / q.px.len() as f64
            });
            let _ = writeln!(csv, "{n};{:.1};{brilho:.1};{dif:.1}", q.t - t0);
            if !sem_fotos {
                salvar_bmp(&saida.join(format!("q{n:03}.bmp")), q);
            }
            ant = Some(&q.px);
        }
        println!("{} quadros em {}", quadros.len(), saida.display());
    }
    Ok(())
}
