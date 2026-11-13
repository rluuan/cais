//! Ícones vetoriais desenhados à mão (Wi-Fi, som, rede, Bluetooth, sol,
//! mídia...). Nada de ícones de terceiros: só linhas e arcos do Direct2D.

use crate::superficie::{arredondado, cor, retangulo};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

type Dc = ID2D1DeviceContext;
type Pincel = ID2D1SolidColorBrush;

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

/// Ponto no círculo de centro (cx, cy): ângulo em graus, 90 = para cima.
fn polar(cx: f32, cy: f32, r: f32, graus: f32) -> D2D_POINT_2F {
    let a = graus.to_radians();
    pt(cx + r * a.cos(), cy - r * a.sin())
}

unsafe fn traco(dc: &Dc) -> Option<ID2D1StrokeStyle> {
    let f = dc.GetFactory().ok()?;
    f.CreateStrokeStyle(
        &D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        },
        None,
    )
    .ok()
}

/// Linha quebrada (aberta) ou polígono (fechado).
unsafe fn caminho(dc: &Dc, pontos: &[(f32, f32)], fechado: bool) -> Option<ID2D1PathGeometry> {
    let g = dc.GetFactory().ok()?.CreatePathGeometry().ok()?;
    let s = g.Open().ok()?;
    let (x, y) = pontos[0];
    s.BeginFigure(
        pt(x, y),
        if fechado { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW },
    );
    for &(x, y) in &pontos[1..] {
        s.AddLine(pt(x, y));
    }
    s.EndFigure(if fechado { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN });
    s.Close().ok()?;
    Some(g)
}

unsafe fn linhas(dc: &Dc, p: &Pincel, pontos: &[(f32, f32)], largura: f32) {
    if let Some(g) = caminho(dc, pontos, false) {
        dc.DrawGeometry(&g, p, largura, traco(dc).as_ref());
    }
}

unsafe fn poligono(dc: &Dc, p: &Pincel, pontos: &[(f32, f32)]) {
    if let Some(g) = caminho(dc, pontos, true) {
        dc.FillGeometry(&g, p, None);
    }
}

/// Arco (só o traço) entre dois ângulos, no sentido horário da tela.
unsafe fn arco(dc: &Dc, p: &Pincel, cx: f32, cy: f32, r: f32, de: f32, ate: f32, largura: f32) {
    let Some(g) = dc.GetFactory().ok().and_then(|f| f.CreatePathGeometry().ok()) else {
        return;
    };
    let Ok(s) = g.Open() else { return };
    s.BeginFigure(polar(cx, cy, r, de), D2D1_FIGURE_BEGIN_HOLLOW);
    s.AddArc(&D2D1_ARC_SEGMENT {
        point: polar(cx, cy, r, ate),
        size: D2D_SIZE_F { width: r, height: r },
        rotationAngle: 0.0,
        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
        arcSize: D2D1_ARC_SIZE_SMALL,
    });
    s.EndFigure(D2D1_FIGURE_END_OPEN);
    if s.Close().is_ok() {
        dc.DrawGeometry(&g, p, largura, traco(dc).as_ref());
    }
}

/// Setor de anel (faixa do leque do Wi-Fi), de 135° a 45°.
unsafe fn setor(dc: &Dc, p: &Pincel, cx: f32, cy: f32, r0: f32, r1: f32) {
    let Some(g) = dc.GetFactory().ok().and_then(|f| f.CreatePathGeometry().ok()) else {
        return;
    };
    let Ok(s) = g.Open() else { return };
    let arco = |r: f32, ate: f32, horario: bool| D2D1_ARC_SEGMENT {
        point: polar(cx, cy, r, ate),
        size: D2D_SIZE_F { width: r, height: r },
        rotationAngle: 0.0,
        sweepDirection: if horario {
            D2D1_SWEEP_DIRECTION_CLOCKWISE
        } else {
            D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
        },
        arcSize: D2D1_ARC_SIZE_SMALL,
    };
    s.BeginFigure(polar(cx, cy, r1, 135.0), D2D1_FIGURE_BEGIN_FILLED);
    s.AddArc(&arco(r1, 45.0, true));
    if r0 > 0.0 {
        s.AddLine(polar(cx, cy, r0, 45.0));
        s.AddArc(&arco(r0, 135.0, false));
    } else {
        s.AddLine(pt(cx, cy));
    }
    s.EndFigure(D2D1_FIGURE_END_CLOSED);
    if s.Close().is_ok() {
        dc.FillGeometry(&g, p, None);
    }
}

/// Leque do Wi-Fi. `acesas` = quantas faixas (0 a 3) aparecem fortes.
/// `t` = tamanho (1.0 cabe na barra do topo).
pub unsafe fn wifi(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32, acesas: u8, rgb: (f32, f32, f32)) {
    let base = cy + 5.5 * t;
    let faixas = [(0.0, 3.6), (5.2, 8.4), (10.0, 13.2)];
    for (i, (r0, r1)) in faixas.iter().enumerate() {
        let a = if (i as u8) < acesas { 0.95 } else { 0.28 };
        p.SetColor(&cor(rgb.0, rgb.1, rgb.2, a));
        setor(dc, p, cx, base, r0 * t, r1 * t);
    }
}

/// Barras de sinal (0 a 5 do Windows) para faixas acesas (1 a 3).
pub fn faixas_do_sinal(barras: u8) -> u8 {
    match barras {
        0..=1 => 1,
        2..=3 => 2,
        _ => 3,
    }
}

/// Alto-falante com ondas conforme o volume (0..1); riscado se mudo.
pub unsafe fn som(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32, volume: f32, mudo: bool, c: D2D1_COLOR_F) {
    p.SetColor(&c);
    let x = cx - 5.0 * t;
    dc.FillRoundedRectangle(
        &arredondado(retangulo(x - 3.2 * t, cy - 2.6 * t, x + 0.6 * t, cy + 2.6 * t), 0.8 * t),
        p,
    );
    poligono(
        dc,
        p,
        &[
            (x, cy - 2.6 * t),
            (x + 4.2 * t, cy - 6.2 * t),
            (x + 4.2 * t, cy + 6.2 * t),
            (x, cy + 2.6 * t),
        ],
    );
    let ox = x + 4.6 * t;
    if mudo {
        let (a, b) = (ox + 3.0 * t, ox + 8.0 * t);
        linhas(dc, p, &[(a, cy - 2.5 * t), (b, cy + 2.5 * t)], 1.4 * t);
        linhas(dc, p, &[(a, cy + 2.5 * t), (b, cy - 2.5 * t)], 1.4 * t);
        return;
    }
    let ondas = if volume <= 0.001 {
        0
    } else if volume < 0.34 {
        1
    } else if volume < 0.67 {
        2
    } else {
        3
    };
    for i in 0..3 {
        let r = (3.0 + i as f32 * 3.2) * t;
        let a = if i < ondas { c.a } else { c.a * 0.28 };
        p.SetColor(&cor(c.r, c.g, c.b, a));
        arco(dc, p, ox - 1.0 * t, cy, r, 50.0, -50.0, 1.4 * t);
    }
}

/// Rede com cabo: três caixinhas ligadas.
pub unsafe fn cabo(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32, c: D2D1_COLOR_F) {
    p.SetColor(&c);
    let caixa = |x: f32, y: f32| arredondado(retangulo(x - 2.8 * t, y - 2.3 * t, x + 2.8 * t, y + 2.3 * t), 1.0 * t);
    dc.DrawRoundedRectangle(&caixa(cx, cy - 4.5 * t), p, 1.2 * t, None);
    dc.DrawRoundedRectangle(&caixa(cx - 5.5 * t, cy + 4.5 * t), p, 1.2 * t, None);
    dc.DrawRoundedRectangle(&caixa(cx + 5.5 * t, cy + 4.5 * t), p, 1.2 * t, None);
    linhas(dc, p, &[(cx, cy - 2.2 * t), (cx, cy)], 1.2 * t);
    linhas(
        dc,
        p,
        &[
            (cx - 5.5 * t, cy + 2.2 * t),
            (cx - 5.5 * t, cy),
            (cx + 5.5 * t, cy),
            (cx + 5.5 * t, cy + 2.2 * t),
        ],
        1.2 * t,
    );
}

/// Runa do Bluetooth.
pub unsafe fn bluetooth(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32, c: D2D1_COLOR_F) {
    p.SetColor(&c);
    let s = t;
    linhas(
        dc,
        p,
        &[
            (cx - 3.6 * s, cy - 3.6 * s),
            (cx + 3.6 * s, cy + 3.6 * s),
            (cx, cy + 7.2 * s),
            (cx, cy - 7.2 * s),
            (cx + 3.6 * s, cy - 3.6 * s),
            (cx - 3.6 * s, cy + 3.6 * s),
        ],
        1.5 * s,
    );
}

pub unsafe fn sol(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32, c: D2D1_COLOR_F) {
    p.SetColor(&c);
    dc.FillEllipse(
        &D2D1_ELLIPSE { point: pt(cx, cy), radiusX: 2.8 * t, radiusY: 2.8 * t },
        p,
    );
    for i in 0..8 {
        let g = i as f32 * 45.0;
        let a = polar(cx, cy, 4.8 * t, g);
        let b = polar(cx, cy, 6.6 * t, g);
        linhas(dc, p, &[(a.x, a.y), (b.x, b.y)], 1.3 * t);
    }
}

pub unsafe fn tocar(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32) {
    poligono(
        dc,
        p,
        &[(cx - 4.0 * t, cy - 6.0 * t), (cx + 6.0 * t, cy), (cx - 4.0 * t, cy + 6.0 * t)],
    );
}

pub unsafe fn pausar(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32) {
    for dx in [-3.2f32, 3.2] {
        dc.FillRoundedRectangle(
            &arredondado(
                retangulo(cx + dx * t - 1.6 * t, cy - 6.0 * t, cx + dx * t + 1.6 * t, cy + 6.0 * t),
                0.8 * t,
            ),
            p,
        );
    }
}

/// Dois triângulos: para frente (`frente`) ou para trás.
pub unsafe fn pular(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32, frente: bool) {
    let d = if frente { 1.0 } else { -1.0 };
    for k in [-1.0f32, 1.0] {
        let x0 = cx + (k * 4.0 - 4.0) * t * d;
        poligono(
            dc,
            p,
            &[(x0, cy - 5.0 * t), (x0 + 8.0 * t * d, cy), (x0, cy + 5.0 * t)],
        );
    }
}

pub unsafe fn marca(dc: &Dc, p: &Pincel, cx: f32, cy: f32) {
    linhas(dc, p, &[(cx - 4.0, cy), (cx - 1.2, cy + 3.0), (cx + 4.5, cy - 3.5)], 1.6);
}

/// Seta ">" (fechado) ou "v" (aberto).
pub unsafe fn seta(dc: &Dc, p: &Pincel, cx: f32, cy: f32, aberto: bool) {
    if aberto {
        linhas(dc, p, &[(cx - 4.0, cy - 2.0), (cx, cy + 2.0), (cx + 4.0, cy - 2.0)], 1.5);
    } else {
        linhas(dc, p, &[(cx - 2.0, cy - 4.0), (cx + 2.0, cy), (cx - 2.0, cy + 4.0)], 1.5);
    }
}

pub unsafe fn lupa(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32, c: D2D1_COLOR_F) {
    p.SetColor(&c);
    dc.DrawEllipse(
        &D2D1_ELLIPSE { point: pt(cx - 1.5 * t, cy - 1.5 * t), radiusX: 5.5 * t, radiusY: 5.5 * t },
        p,
        1.7 * t,
        None,
    );
    linhas(dc, p, &[(cx + 2.6 * t, cy + 2.6 * t), (cx + 6.5 * t, cy + 6.5 * t)], 2.0 * t);
}

/// Engrenagem (Configurações).
pub unsafe fn engrenagem(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32, c: D2D1_COLOR_F) {
    p.SetColor(&c);
    for i in 0..8 {
        let g = i as f32 * 45.0;
        let a = polar(cx, cy, 5.0 * t, g);
        let b = polar(cx, cy, 8.0 * t, g);
        linhas(dc, p, &[(a.x, a.y), (b.x, b.y)], 2.6 * t);
    }
    dc.DrawEllipse(&D2D1_ELLIPSE { point: pt(cx, cy), radiusX: 5.0 * t, radiusY: 5.0 * t }, p, 2.2 * t, None);
}

/// Calculadora: um quadrado com "=".
pub unsafe fn calculadora(dc: &Dc, p: &Pincel, cx: f32, cy: f32, t: f32, c: D2D1_COLOR_F) {
    p.SetColor(&c);
    dc.DrawRoundedRectangle(
        &arredondado(retangulo(cx - 8.0 * t, cy - 8.0 * t, cx + 8.0 * t, cy + 8.0 * t), 3.0 * t),
        p,
        1.5 * t,
        None,
    );
    linhas(dc, p, &[(cx - 4.0 * t, cy - 2.0 * t), (cx + 4.0 * t, cy - 2.0 * t)], 1.6 * t);
    linhas(dc, p, &[(cx - 4.0 * t, cy + 2.5 * t), (cx + 4.0 * t, cy + 2.5 * t)], 1.6 * t);
}

/// Ícone do Launchpad: azulejo escuro com uma grade de 3x3 pontos coloridos.
pub unsafe fn launchpad(dc: &Dc, p: &Pincel, r: D2D_RECT_F, opacidade: f32) {
    let t = r.right - r.left;
    p.SetColor(&cor(0.20, 0.21, 0.26, opacidade));
    dc.FillRoundedRectangle(&arredondado(r, t * 0.225), p);
    p.SetColor(&cor(1.0, 1.0, 1.0, 0.12 * opacidade));
    dc.DrawRoundedRectangle(&arredondado(r, t * 0.225), p, 1.0, None);
    let cores = [
        (1.0, 0.37, 0.34), (1.0, 0.62, 0.2), (1.0, 0.84, 0.25),
        (0.36, 0.84, 0.45), (0.25, 0.75, 0.95), (0.2, 0.52, 1.0),
        (0.58, 0.4, 0.98), (0.95, 0.4, 0.75), (0.75, 0.75, 0.8),
    ];
    let passo = t * 0.22;
    let lado = t * 0.15;
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    for (k, (cr, cg, cb)) in cores.iter().enumerate() {
        let x = cx + ((k % 3) as f32 - 1.0) * passo;
        let y = cy + ((k / 3) as f32 - 1.0) * passo;
        p.SetColor(&cor(*cr, *cg, *cb, opacidade));
        dc.FillRoundedRectangle(
            &arredondado(retangulo(x - lado / 2.0, y - lado / 2.0, x + lado / 2.0, y + lado / 2.0), lado * 0.3),
            p,
        );
    }
}
/// Ícone do Finder do Cais: azulejo azul com uma pasta branca (desenho
/// próprio, não o rosto do Finder da Apple).
pub unsafe fn finder(dc: &Dc, p: &Pincel, r: D2D_RECT_F, opacidade: f32) {
    let t = r.right - r.left;
    let raio = t * 0.225;
    p.SetColor(&cor(0.10, 0.45, 0.95, opacidade));
    dc.FillRoundedRectangle(&arredondado(r, raio), p);
    p.SetColor(&cor(0.30, 0.65, 1.0, opacidade));
    dc.FillRoundedRectangle(&arredondado(retangulo(r.left, r.top, r.right, r.top + t * 0.5), raio), p);
    p.SetColor(&cor(0.10, 0.45, 0.95, opacidade));
    dc.FillRectangle(&retangulo(r.left, r.top + t * 0.3, r.right, r.top + t * 0.5), p);
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0 + t * 0.03);
    let (l, a) = (t * 0.27, t * 0.19);
    p.SetColor(&cor(1.0, 1.0, 1.0, 0.96 * opacidade));
    poligono(
        dc,
        p,
        &[
            (cx - l, cy - a - t * 0.06),
            (cx - l * 0.25, cy - a - t * 0.06),
            (cx - l * 0.05, cy - a),
            (cx + l, cy - a),
            (cx + l, cy + a),
            (cx - l, cy + a),
        ],
    );
    p.SetColor(&cor(0.10, 0.45, 0.95, 0.35 * opacidade));
    dc.FillRectangle(&retangulo(cx - l, cy - a + t * 0.05, cx + l, cy - a + t * 0.07), p);
}

/// Lixeira do dock: um cesto de tela, cheio de papéis quando tem algo.
pub unsafe fn lixeira(dc: &Dc, p: &Pincel, r: D2D_RECT_F, opacidade: f32, cheia: bool) {
    let t = r.right - r.left;
    let cx = (r.left + r.right) / 2.0;
    let (topo, base) = (r.top + t * 0.16, r.bottom - t * 0.06);
    let (lt, lb) = (t * 0.36, t * 0.29);
    // Papéis amassados saindo pela boca.
    if cheia {
        for (dx, dy, raio) in [(-0.13, 0.02, 0.11), (0.06, -0.02, 0.12), (0.17, 0.04, 0.09), (-0.02, 0.06, 0.1)] {
            p.SetColor(&cor(0.97, 0.97, 0.95, opacidade));
            dc.FillEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx + dx * t, y: topo + dy * t },
                    radiusX: raio * t,
                    radiusY: raio * t * 0.85,
                },
                p,
            );
            p.SetColor(&cor(0.6, 0.6, 0.62, 0.6 * opacidade));
            dc.DrawEllipse(
                &D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx + dx * t, y: topo + dy * t },
                    radiusX: raio * t,
                    radiusY: raio * t * 0.85,
                },
                p,
                0.8,
                None,
            );
        }
    }
    // Corpo de vidro fosco.
    p.SetColor(&cor(0.86, 0.88, 0.92, 0.55 * opacidade));
    poligono(dc, p, &[(cx - lt, topo), (cx + lt, topo), (cx + lb, base), (cx - lb, base)]);
    // Grade da tela.
    p.SetColor(&cor(0.35, 0.37, 0.42, 0.55 * opacidade));
    for k in 1..6 {
        let f = k as f32 / 6.0;
        let (xt, xb) = (cx - lt + 2.0 * lt * f, cx - lb + 2.0 * lb * f);
        dc.DrawLine(D2D_POINT_2F { x: xt, y: topo }, D2D_POINT_2F { x: xb, y: base }, p, 0.9, None);
    }
    for k in 1..5 {
        let f = k as f32 / 5.0;
        let y = topo + (base - topo) * f;
        let meia = lt + (lb - lt) * f;
        dc.DrawLine(D2D_POINT_2F { x: cx - meia, y }, D2D_POINT_2F { x: cx + meia, y }, p, 0.9, None);
    }
    // Aro de cima e contorno.
    p.SetColor(&cor(0.72, 0.74, 0.79, opacidade));
    dc.FillRoundedRectangle(&arredondado(retangulo(cx - lt - t * 0.03, topo - t * 0.035, cx + lt + t * 0.03, topo + t * 0.035), t * 0.03), p);
    p.SetColor(&cor(0.45, 0.47, 0.52, 0.8 * opacidade));
    dc.DrawLine(D2D_POINT_2F { x: cx - lt, y: topo }, D2D_POINT_2F { x: cx - lb, y: base }, p, 1.2, None);
    dc.DrawLine(D2D_POINT_2F { x: cx + lt, y: topo }, D2D_POINT_2F { x: cx + lb, y: base }, p, 1.2, None);
    dc.DrawLine(D2D_POINT_2F { x: cx - lb, y: base }, D2D_POINT_2F { x: cx + lb, y: base }, p, 1.2, None);
}