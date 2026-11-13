//! Vidro fosco no formato exato do dock: o compositor do Windows borra o que
//! está atrás da janela (na GPU) e nós só recortamos num retângulo arredondado.

use windows::core::{Interface, Result};
use windows::Foundation::Numerics::{Vector2, Vector3};
use windows::UI::Composition::*;

pub struct Vidro {
    grupo: ContainerVisual,
    forma: CompositionRoundedRectangleGeometry,
    id: u64,
}

/// Tinta por cima do blur (cor do tema, para ter contraste em qualquer papel
/// de parede). O dock, a barra do topo e os painéis têm cada um a sua.
pub use crate::tema::Tinta;
pub const TINTA_DOCK: Tinta = Tinta::Dock;
pub const TINTA_TOPO: Tinta = Tinta::Topo;
pub const TINTA_PAINEL: Tinta = Tinta::Painel;

impl Drop for Vidro {
    fn drop(&mut self) {
        crate::tema::esquecer(self.id);
    }
}

impl Vidro {
    pub fn new(comp: &Compositor, raiz: &ContainerVisual, tipo: Tinta) -> Result<Self> {
        let grupo = comp.CreateContainerVisual()?;
        let inteiro = Vector2 { X: 1.0, Y: 1.0 };

        let fundo = comp.CreateSpriteVisual()?;
        fundo.SetBrush(&comp.CreateHostBackdropBrush()?)?;
        fundo.SetRelativeSizeAdjustment(inteiro)?;

        let tinta = comp.CreateSpriteVisual()?;
        let pincel = comp.CreateColorBrushWithColor(crate::tema::cor_vidro(tipo))?;
        tinta.SetBrush(&pincel)?;
        let id = crate::tema::registrar(tipo, pincel);
        tinta.SetRelativeSizeAdjustment(inteiro)?;

        let filhos = grupo.Children()?;
        filhos.InsertAtTop(&fundo)?;
        filhos.InsertAtTop(&tinta)?;

        let forma = comp.CreateRoundedRectangleGeometry()?;
        let recorte = comp.CreateGeometricClipWithGeometry(&forma)?;
        grupo.SetClip(&recorte)?;

        raiz.Children()?.InsertAtBottom(&grupo.cast::<Visual>()?)?;
        Ok(Self { grupo, forma, id })
    }

    /// Posição e tamanho em pixels físicos.
    pub fn definir(&self, x: f32, y: f32, l: f32, a: f32, raio: f32) -> Result<()> {
        self.grupo.SetOffset(Vector3 { X: x, Y: y, Z: 0.0 })?;
        self.grupo.SetSize(Vector2 { X: l, Y: a })?;
        self.forma.SetSize(Vector2 { X: l, Y: a })?;
        self.forma.SetCornerRadius(Vector2 { X: raio, Y: raio })?;
        Ok(())
    }
}
