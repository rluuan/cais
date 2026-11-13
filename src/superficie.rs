//! Uma janela + swap chain, montada no compositor do Windows. Só apresenta
//! quando alguém pede um quadro: nada de loop de renderização rodando à toa.

use crate::gpu::Gpu;
use crate::janela;
use windows::core::{Interface, Result, HSTRING};
use windows::Foundation::Numerics::{Vector2, Vector3};
use windows::Foundation::TimeSpan;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;

pub struct Superficie {
    pub hwnd: HWND,
    pub dpi: f32,
    /// Camadas da janela: outras (como o vidro) entram por baixo do conteúdo.
    pub raiz: ContainerVisual,
    swap: IDXGISwapChain1,
    bitmap: Option<ID2D1Bitmap1>,
    conteudo: SpriteVisual,
    _alvo: DesktopWindowTarget,
    /// Recorte de cantos arredondados do conteúdo (raio em DIPs).
    canto: Option<(CompositionRoundedRectangleGeometry, f32)>,
}

impl Superficie {
    pub fn new(gpu: &Gpu, hwnd: HWND) -> Result<Self> {
        let (l, a) = janela::tamanho_cliente(hwnd);
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: l,
            Height: a,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            Flags: 0,
        };
        unsafe {
            let swap =
                gpu.dxgi_factory
                    .CreateSwapChainForComposition(&gpu.dxgi_device, &desc, None)?;

            let interop: ICompositorDesktopInterop = gpu.comp.cast()?;
            let alvo = interop.CreateDesktopWindowTarget(hwnd, true)?;
            let raiz = gpu.comp.CreateContainerVisual()?;
            alvo.SetRoot(&raiz)?;

            let sup = gpu.comp_interop.CreateCompositionSurfaceForSwapChain(&swap)?;
            let pincel = gpu.comp.CreateSurfaceBrushWithSurface(&sup)?;
            pincel.SetStretch(CompositionStretch::None)?;
            let conteudo = gpu.comp.CreateSpriteVisual()?;
            conteudo.SetBrush(&pincel)?;
            conteudo.SetSize(Vector2 {
                X: l as f32,
                Y: a as f32,
            })?;
            raiz.Children()?.InsertAtTop(&conteudo.cast::<Visual>()?)?;

            Ok(Self {
                hwnd,
                dpi: janela::escala(hwnd) * 96.0,
                raiz,
                swap,
                bitmap: None,
                conteudo,
                _alvo: alvo,
                canto: None,
            })
        }
    }

    pub fn redimensionar(&mut self, gpu: &Gpu) -> Result<()> {
        self.bitmap = None;
        let (l, a) = janela::tamanho_cliente(self.hwnd);
        unsafe {
            gpu.d2d_dc.SetTarget(None);
            self.swap
                .ResizeBuffers(0, l, a, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
        }
        self.conteudo.SetSize(Vector2 {
            X: l as f32,
            Y: a as f32,
        })?;
        self.dpi = janela::escala(self.hwnd) * 96.0;
        if let Some((g, raio)) = &self.canto {
            let r = raio * self.dpi / 96.0;
            g.SetSize(Vector2 { X: l as f32, Y: a as f32 })?;
            g.SetCornerRadius(Vector2 { X: r, Y: r })?;
        }
        Ok(())
    }

    /// Recorta o que for desenhado com cantos arredondados (raio em DIPs),
    /// como as janelas do Mac. O vidro tem o próprio recorte.
    pub fn arredondar(&mut self, gpu: &Gpu, raio: f32) -> Result<()> {
        let (l, a) = janela::tamanho_cliente(self.hwnd);
        let g = gpu.comp.CreateRoundedRectangleGeometry()?;
        let r = raio * self.dpi / 96.0;
        g.SetSize(Vector2 { X: l as f32, Y: a as f32 })?;
        g.SetCornerRadius(Vector2 { X: r, Y: r })?;
        let clip = gpu.comp.CreateGeometricClipWithGeometry(&g)?;
        self.conteudo.SetClip(&clip)?;
        self.canto = Some((g, raio));
        Ok(())
    }

    /// Aparece com fade, descendo `dy` px e/ou crescendo de `escala_ini`
    /// até 1. Quem anima é o compositor do Windows: nenhum quadro nosso.
    pub fn animar_entrada(&self, gpu: &Gpu, dy: f32, escala_ini: f32) {
        let _ = (|| -> Result<()> {
            let dur = TimeSpan { Duration: 1_700_000 }; // 170 ms
            let op = gpu.comp.CreateScalarKeyFrameAnimation()?;
            op.InsertKeyFrame(0.0, 0.0)?;
            op.InsertKeyFrame(1.0, 1.0)?;
            op.SetDuration(dur)?;
            self.raiz.StartAnimation(&HSTRING::from("Opacity"), &op)?;
            if dy != 0.0 {
                let mv = gpu.comp.CreateVector3KeyFrameAnimation()?;
                mv.InsertKeyFrame(0.0, Vector3 { X: 0.0, Y: dy, Z: 0.0 })?;
                mv.InsertKeyFrame(1.0, Vector3 { X: 0.0, Y: 0.0, Z: 0.0 })?;
                mv.SetDuration(dur)?;
                self.raiz.StartAnimation(&HSTRING::from("Offset"), &mv)?;
            }
            if escala_ini != 1.0 {
                let (l, a) = janela::tamanho_cliente(self.hwnd);
                self.raiz.SetCenterPoint(Vector3 { X: l as f32 / 2.0, Y: a as f32 / 2.0, Z: 0.0 })?;
                let es = gpu.comp.CreateVector3KeyFrameAnimation()?;
                es.InsertKeyFrame(0.0, Vector3 { X: escala_ini, Y: escala_ini, Z: 1.0 })?;
                es.InsertKeyFrame(1.0, Vector3 { X: 1.0, Y: 1.0, Z: 1.0 })?;
                es.SetDuration(dur)?;
                self.raiz.StartAnimation(&HSTRING::from("Scale"), &es)?;
            }
            Ok(())
        })();
    }

    /// Desenha um quadro (coordenadas em DIPs) e apresenta.
    pub fn desenhar(&mut self, gpu: &Gpu, f: impl FnOnce(&ID2D1DeviceContext)) -> Result<()> {
        let dc = &gpu.d2d_dc;
        unsafe {
            if self.bitmap.is_none() {
                let props = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: self.dpi,
                    dpiY: self.dpi,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    ..Default::default()
                };
                let surface: IDXGISurface = self.swap.GetBuffer(0)?;
                self.bitmap = Some(dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?);
            }
            dc.SetTarget(self.bitmap.as_ref().unwrap());
            dc.SetDpi(self.dpi, self.dpi);
            dc.BeginDraw();
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            f(dc);
            let r = dc.EndDraw(None, None);
            dc.SetTarget(None);
            r?;
            self.swap.Present(1, DXGI_PRESENT(0)).ok()
        }
    }
}

pub fn cor(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

pub fn retangulo(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

pub fn arredondado(rect: D2D_RECT_F, raio: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: raio,
        radiusY: raio,
    }
}
