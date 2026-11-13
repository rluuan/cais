//! Dispositivos gráficos compartilhados por todas as janelas do Cais:
//! D3D11 + Direct2D para desenhar, e o compositor do Windows para montar
//! as camadas (inclusive o vidro com blur) na GPU.

use windows::core::{Interface, Result};
use windows::System::DispatcherQueueController;
use windows::UI::Composition::Compositor;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::WinRT::Composition::ICompositorInterop;
use windows::Win32::System::WinRT::*;

pub struct Gpu {
    pub dxgi_device: IDXGIDevice,
    pub dxgi_factory: IDXGIFactory2,
    pub d2d_dc: ID2D1DeviceContext,
    pub dwrite: IDWriteFactory,
    pub comp: Compositor,
    pub comp_interop: ICompositorInterop,
    _fila: DispatcherQueueController,
    _d3d: ID3D11Device,
    _d2d_device: ID2D1Device,
}

impl Gpu {
    pub fn new() -> Result<Self> {
        unsafe {
            let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
            let mut d3d = None;
            let hw = D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            );
            if hw.is_err() {
                // Sem GPU utilizável: renderizador por software (WARP).
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_WARP,
                    HMODULE::default(),
                    flags,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut d3d),
                    None,
                    None,
                )?;
            }
            let d3d: ID3D11Device = d3d.unwrap();

            let dxgi_device: IDXGIDevice = d3d.cast()?;
            // No máximo 1 quadro na fila: animação responde na hora ao mouse.
            let dxgi1: IDXGIDevice1 = d3d.cast()?;
            dxgi1.SetMaximumFrameLatency(1)?;

            let adapter = dxgi_device.GetAdapter()?;
            let dxgi_factory: IDXGIFactory2 = adapter.GetParent()?;

            let d2d_factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d_factory.CreateDevice(&dxgi_device)?;
            let d2d_dc = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;

            // O compositor precisa de uma fila de despacho nesta thread
            // (ela usa o mesmo laço de mensagens do app, sem thread extra).
            let fila = CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            })?;
            let comp = Compositor::new()?;
            let comp_interop: ICompositorInterop = comp.cast()?;

            Ok(Self {
                dxgi_device,
                dxgi_factory,
                d2d_dc,
                dwrite,
                comp,
                comp_interop,
                _fila: fila,
                _d3d: d3d,
                _d2d_device: d2d_device,
            })
        }
    }
}
