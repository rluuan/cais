//! Animação por física de mola: movimento natural, com um leve assentamento,
//! e que para sozinha (aí o app volta a gastar zero).

#[derive(Clone, Copy, Default)]
pub struct Mola {
    pub x: f32,
    v: f32,
    pub alvo: f32,
}

const RIGIDEZ: f32 = 260.0;
const AMORTECIMENTO: f32 = 27.0; // ~0,84 do crítico: assenta sem quicar demais

impl Mola {
    pub fn passo(&mut self, dt: f32) {
        let mut resta = dt;
        while resta > 0.0 {
            let h = resta.min(1.0 / 240.0);
            let a = -RIGIDEZ * (self.x - self.alvo) - AMORTECIMENTO * self.v;
            self.v += a * h;
            self.x += self.v * h;
            resta -= h;
        }
        if self.parada() {
            self.x = self.alvo;
            self.v = 0.0;
        }
    }

    pub fn parada(&self) -> bool {
        (self.x - self.alvo).abs() < 0.0005 && self.v.abs() < 0.0005
    }
}
