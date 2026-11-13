//! Idioma da interface: português ou inglês. Por padrão segue o idioma do
//! Windows (português se ele estiver em português, inglês nos outros casos).
//!
//! Uso: `t("Texto em português", "English text")`.

use std::sync::atomic::{AtomicBool, Ordering};

static INGLES: AtomicBool = AtomicBool::new(false);

/// Escolhas da Preferência "Idioma": (rótulo, valor no config).
pub const IDIOMAS: [(&str, &str); 3] = [("Automático", "automatico"), ("Português", "pt"), ("English", "en")];

#[link(name = "kernel32")]
extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
}

/// O Windows está em português (qualquer variante)?
pub fn windows_em_portugues() -> bool {
    let lang = unsafe { GetUserDefaultUILanguage() };
    lang & 0x3ff == 0x16 // LANG_PORTUGUESE
}

/// Aplica a escolha do config ("automatico", "pt" ou "en"). Devolve se mudou.
pub fn configurar(escolha: &str) -> bool {
    let en = match escolha {
        "pt" => false,
        "en" => true,
        _ => !windows_em_portugues(),
    };
    INGLES.swap(en, Ordering::Relaxed) != en
}

pub fn ingles() -> bool {
    INGLES.load(Ordering::Relaxed)
}

/// Escolhe o texto do idioma atual.
pub fn t(pt: &'static str, en: &'static str) -> &'static str {
    if ingles() {
        en
    } else {
        pt
    }
}

/// Rótulos curtos das tabelas de opções (temas, cores, layouts...): o
/// texto em português é a chave. Em português devolve o próprio texto.
pub fn rotulo(pt: &'static str) -> &'static str {
    if !ingles() {
        return pt;
    }
    match pt {
        "Automática" | "Automático" => "Automatic",
        "Escura" => "Dark",
        "Clara" => "Light",
        "Azul" => "Blue",
        "Roxo" => "Purple",
        "Rosa" => "Pink",
        "Vermelho" => "Red",
        "Laranja" => "Orange",
        "Amarelo" => "Yellow",
        "Verde" => "Green",
        "Grafite" => "Graphite",
        "A do Windows" => "Windows accent",
        "Pouca" => "Low",
        "Média" => "Medium",
        "Muita" => "High",
        "Nenhum" => "None",
        "Pequeno" => "Small",
        "Médio" => "Medium",
        "Grande" => "Large",
        "2 colunas" => "2 columns",
        "3 colunas" => "3 columns",
        "Grade 2x2 (4 janelas)" => "2x2 grid (4 windows)",
        "Grade 3x2 (6 janelas)" => "3x2 grid (6 windows)",
        "Grade 3x3 (9 janelas)" => "3x3 grid (9 windows)",
        "Uma grande e duas pequenas" => "One large and two small",
        "Uma grande e quatro pequenas" => "One large and four small",
        "Sim" => "Yes",
        "Não" => "No",
        "Português" => "Português",
        "Esquerda" => "Left",
        "Direita" => "Right",
        "Embaixo" => "Bottom",
        outro => outro,
    }
}

/// Texto em UTF-16 terminado em zero (para APIs do Windows).
pub fn largo(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Nome de localidade para o DirectWrite.
pub fn localidade() -> windows::core::PCWSTR {
    if ingles() {
        windows::core::w!("en-US")
    } else {
        windows::core::w!("pt-BR")
    }
}

/// Os testes que mudam o idioma (global) rodam um de cada vez.
#[cfg(test)]
pub static TRAVA_DE_TESTE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn escolhe_o_idioma() {
        let _t = TRAVA_DE_TESTE.lock().unwrap_or_else(|e| e.into_inner());
        configurar("en");
        assert_eq!(t("Sair", "Quit"), "Quit");
        configurar("pt");
        assert_eq!(t("Sair", "Quit"), "Sair");
    }
}
