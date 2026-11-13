//! Cantos ativos: encostar o mouse num canto da tela dispara uma ação.
//! A detecção fica em `entrada.rs`; aqui ficam as ações e os nomes.

/// (valor no config, nome em português, nome em inglês)
pub const ACOES: [(&str, &str, &str); 6] = [
    ("nada", "Nada", "Nothing"),
    ("visao", "Visão Geral", "Overview"),
    ("mesa", "Mostrar a área de trabalho", "Show desktop"),
    ("apps", "Apps", "Apps"),
    ("busca", "Busca", "Search"),
    ("bloquear", "Bloquear a tela", "Lock screen"),
];

/// Nomes dos cantos (para as Preferências).
pub const NOMES: [(&str, &str); 4] = [
    ("Canto superior esquerdo", "Top-left corner"),
    ("Canto superior direito", "Top-right corner"),
    ("Canto inferior esquerdo", "Bottom-left corner"),
    ("Canto inferior direito", "Bottom-right corner"),
];

pub fn indice(valor: &str) -> usize {
    ACOES.iter().position(|a| a.0 == valor).unwrap_or(0)
}

pub fn nome(i: usize) -> &'static str {
    let a = ACOES[i.min(ACOES.len() - 1)];
    crate::idioma::t(a.1, a.2)
}

/// Algum canto faz alguma coisa?
pub fn algum_ligado(cantos: &[String; 4]) -> bool {
    cantos.iter().any(|c| c != "nada")
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn indices_das_acoes() {
        assert_eq!(indice("nada"), 0);
        assert_eq!(indice("visao"), 1);
        assert_eq!(indice("inexistente"), 0);
        let mut c: [String; 4] = Default::default();
        c.iter_mut().for_each(|s| *s = "nada".into());
        assert!(!algum_ligado(&c));
        c[2] = "mesa".into();
        assert!(algum_ligado(&c));
    }
}
