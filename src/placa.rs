//! Ícones uniformes: todo ícone de app vira um "quadrado arredondado" do
//! mesmo tamanho, com sombra leve, como no Mac. Os ícones do Windows vêm de
//! todo formato (redondos, quadrados, soltos, sem fundo); lado a lado no
//! dock ficam desencontrados.
//!
//! - Ícone que já é um quadrado cheio: é recortado no formato da placa.
//! - Ícone solto (redondo, irregular): vai menor, centralizado numa placa clara.
//!
//! Tudo nos pixels (BGRA pré-multiplicado), antes de virar bitmap da GPU.

use crate::apps::Pixels;
use std::sync::atomic::{AtomicBool, Ordering};

static LIGADO: AtomicBool = AtomicBool::new(true);

pub fn ligar(sim: bool) {
    LIGADO.store(sim, Ordering::Relaxed);
}

pub fn ligado() -> bool {
    LIGADO.load(Ordering::Relaxed)
}

/// Margem da placa dentro do quadro (fração do lado) e expoente da
/// "superelipse" (5 deixa os cantos como os do Mac, mais suaves que um
/// retângulo arredondado comum).
const MARGEM: f32 = 0.06;
const EXPOENTE: f32 = 5.0;
/// Ícone solto: quanto da placa ele ocupa.
const DENTRO: f32 = 0.70;

/// Aplica a placa se a opção estiver ligada.
pub fn aplicar(p: Pixels) -> Pixels {
    if ligado() {
        uniformizar(&p)
    } else {
        p
    }
}

/// Quanto o pixel (x, y) está dentro da superelipse de centro c e meia
/// largura h (1 = dentro, 0 = fora, com 1 px de borda suave).
fn dentro_da_placa(x: f32, y: f32, c: f32, h: f32) -> f32 {
    let u = ((x - c) / h).abs();
    let v = ((y - c) / h).abs();
    let d = (u.powf(EXPOENTE) + v.powf(EXPOENTE)).powf(1.0 / EXPOENTE);
    ((1.0 - d) * h + 0.5).clamp(0.0, 1.0)
}

/// Lê um pixel com interpolação (fora da imagem = transparente).
fn amostrar(p: &Pixels, x: f32, y: f32) -> [f32; 4] {
    let (l, a, d) = (p.0 as i32, p.1 as i32, &p.2);
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |xx: i32, yy: i32| -> [f32; 4] {
        if xx < 0 || yy < 0 || xx >= l || yy >= a {
            return [0.0; 4];
        }
        let i = ((yy * l + xx) * 4) as usize;
        [d[i] as f32, d[i + 1] as f32, d[i + 2] as f32, d[i + 3] as f32]
    };
    let (a00, a10, a01, a11) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
    let mut r = [0.0; 4];
    for k in 0..4 {
        let cima = a00[k] + (a10[k] - a00[k]) * fx;
        let baixo = a01[k] + (a11[k] - a01[k]) * fx;
        r[k] = cima + (baixo - cima) * fy;
    }
    r
}

/// Retângulo dos pixels visíveis (x0, y0, x1, y1) e quanto dele está cheio.
fn contorno(p: &Pixels) -> Option<(u32, u32, u32, u32, f32)> {
    let (l, a, d) = (p.0, p.1, &p.2);
    let (mut x0, mut y0, mut x1, mut y1) = (l, a, 0u32, 0u32);
    for y in 0..a {
        for x in 0..l {
            if d[((y * l + x) * 4 + 3) as usize] > 24 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut cheios = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            if d[((y * l + x) * 4 + 3) as usize] > 200 {
                cheios += 1;
            }
        }
    }
    let area = ((x1 - x0) * (y1 - y0)) as f32;
    Some((x0, y0, x1, y1, cheios as f32 / area))
}

/// O ícone já é um "quadrado" cheio (ocupa o quadro e quase não tem
/// buracos)? Um círculo preenche ~78% do quadrado em volta; um quadrado de
/// cantos arredondados, mais de 90%.
pub fn eh_quadrado(p: &Pixels) -> bool {
    let Some((x0, y0, x1, y1, cheio)) = contorno(p) else { return false };
    let (w, h) = ((x1 - x0) as f32, (y1 - y0) as f32);
    let lado = p.0.min(p.1) as f32;
    cheio >= 0.88 && w >= lado * 0.7 && h >= lado * 0.7 && (w / h - 1.0).abs() < 0.12
}

pub fn uniformizar(p: &Pixels) -> Pixels {
    let n = p.0.min(p.1);
    if n < 8 {
        return p.clone();
    }
    let nf = n as f32;
    let c = nf / 2.0;
    let h = nf * (0.5 - MARGEM);
    let quadrado = eh_quadrado(p);
    let (bx0, by0, bx1, by1) = match contorno(p) {
        Some((x0, y0, x1, y1, _)) => (x0 as f32, y0 as f32, x1 as f32, y1 as f32),
        None => return p.clone(),
    };
    // Mapeamento do quadro de saída para a imagem original.
    let (escala, ox, oy) = if quadrado {
        // O conteúdo ocupa a placa inteira (um pouco além, para não sobrar
        // fresta nos cantos).
        let lado = (bx1 - bx0).max(by1 - by0);
        let s = lado / (2.0 * h * 1.02);
        (s, (bx0 + bx1) / 2.0, (by0 + by1) / 2.0)
    } else {
        let lado = (bx1 - bx0).max(by1 - by0);
        let s = lado / (2.0 * h * DENTRO);
        (s, (bx0 + bx1) / 2.0, (by0 + by1) / 2.0)
    };
    let mut out = vec![0u8; (n * n * 4) as usize];
    let sombra_dy = nf * 0.02;
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Sombra: a mesma forma, um pouco abaixo e espalhada.
            let sombra = {
                let u = ((fx - c) / (h * 1.02)).abs();
                let v = ((fy - c - sombra_dy) / (h * 1.02)).abs();
                let d = (u.powf(EXPOENTE) + v.powf(EXPOENTE)).powf(1.0 / EXPOENTE);
                (((1.06 - d) / 0.1).clamp(0.0, 1.0)) * 0.28
            };
            let forma = dentro_da_placa(fx, fy, c, h);
            // Cor da placa (BGRA pré-multiplicado, 0-255) por baixo do ícone.
            let mut cor = [0.0f32; 4];
            if !quadrado && forma > 0.0 {
                // Branco que escurece de leve para baixo, como o Mac.
                let t = ((fy - (c - h)) / (2.0 * h)).clamp(0.0, 1.0);
                let g = 252.0 - 16.0 * t;
                cor = [g * forma, g * forma, g * forma, 255.0 * forma];
            }
            // O ícone em si.
            let sx = ox + (fx - c) * escala - 0.5;
            let sy = oy + (fy - c) * escala - 0.5;
            let mut ic = amostrar(p, sx, sy);
            if quadrado {
                for k in &mut ic {
                    *k *= forma;
                }
            }
            // Ícone por cima da placa (pré-multiplicado: c = i + p * (1 - ai)).
            let ai = ic[3] / 255.0;
            let mut px = [0.0f32; 4];
            for k in 0..4 {
                px[k] = ic[k] + cor[k] * (1.0 - ai);
            }
            // Filete fino na borda da placa (dá acabamento no fundo claro).
            if !quadrado && forma > 0.0 && forma < 1.0 {
                for k in 0..3 {
                    px[k] *= 0.82 + 0.18 * forma;
                }
            }
            // Tudo por cima da sombra.
            let a = px[3] / 255.0;
            px[3] += 255.0 * sombra * (1.0 - a);
            let i = ((y * n + x) * 4) as usize;
            for k in 0..4 {
                out[i + k] = px[k].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    (n, n, out)
}

#[cfg(test)]
mod testes {
    use super::*;

    fn imagem(n: u32, cheio: impl Fn(f32, f32) -> bool) -> Pixels {
        let mut v = vec![0u8; (n * n * 4) as usize];
        for y in 0..n {
            for x in 0..n {
                if cheio(x as f32 + 0.5, y as f32 + 0.5) {
                    let i = ((y * n + x) * 4) as usize;
                    v[i..i + 4].copy_from_slice(&[200, 100, 50, 255]);
                }
            }
        }
        (n, n, v)
    }

    #[test]
    fn reconhece_quadrado_e_circulo() {
        let quadrado = imagem(64, |x, y| x > 4.0 && x < 60.0 && y > 4.0 && y < 60.0);
        let circulo = imagem(64, |x, y| (x - 32.0).powi(2) + (y - 32.0).powi(2) < 28.0f32.powi(2));
        let pequeno = imagem(64, |x, y| x > 24.0 && x < 40.0 && y > 24.0 && y < 40.0);
        assert!(eh_quadrado(&quadrado));
        assert!(!eh_quadrado(&circulo));
        assert!(!eh_quadrado(&pequeno));
    }

    #[test]
    fn circulo_ganha_placa_e_cantos_ficam_vazios() {
        let circulo = imagem(64, |x, y| (x - 32.0).powi(2) + (y - 32.0).powi(2) < 28.0f32.powi(2));
        let r = uniformizar(&circulo);
        let alfa = |x: u32, y: u32| r.2[((y * 64 + x) * 4 + 3) as usize];
        // Perto do canto da placa (fora do círculo original) agora é placa.
        assert!(alfa(10, 10) > 200);
        // O canto do quadro continua transparente (a placa é arredondada).
        assert!(alfa(0, 0) < 40);
        // O centro continua sendo o ícone.
        let i = ((32 * 64 + 32) * 4) as usize;
        assert_eq!(&r.2[i..i + 4], &[200, 100, 50, 255]);
    }

    #[test]
    fn quadrado_cheio_e_recortado_nos_cantos() {
        let cheio = imagem(64, |_, _| true);
        let r = uniformizar(&cheio);
        let alfa = |x: u32, y: u32| r.2[((y * 64 + x) * 4 + 3) as usize];
        assert!(alfa(32, 32) == 255);
        assert!(alfa(1, 1) < 80, "canto {}", alfa(1, 1));
    }
}
