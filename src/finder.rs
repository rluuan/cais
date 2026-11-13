//! Finder do Cais: gerenciador de arquivos com barra lateral (Favoritos e
//! discos), ícones ou lista, busca, pré-visualização (Espaço), renomear
//! (F2) e o menu de contexto do próprio Windows. Atualiza sozinho quando a
//! pasta muda (o Windows avisa) e só desenha quando algo muda.

use crate::arquivos::{self, Entrada, Escolha, MapaSoltar, ServicoIcones, Soltar, Vigia};
use std::cell::RefCell;
use std::rc::Rc;
use crate::campo::{Campo, Tecla};
use crate::gpu::Gpu;
use crate::icones;
use crate::janela;
use crate::mola::Mola;
use crate::superficie::{arredondado, cor, retangulo, Superficie};
use crate::vidro::{Vidro, TINTA_PAINEL};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_FINDER_ICONES: u32 = WM_APP + 9;
pub const WM_FINDER_PASTA: u32 = WM_APP + 10;
pub const WM_FINDER_BUSCA: u32 = WM_APP + 11;
/// Quem recebe arquivos soltos avisa que a pasta destacada mudou.
pub const WM_FINDER_SOLTAR: u32 = WM_APP + 13;
/// A extração de um .zip terminou (wParam = deu certo).
pub const WM_FINDER_ZIP: u32 = WM_APP + 17;
const TIMER_RECARREGAR: usize = 7;

const TOPO: f32 = 52.0;
/// Altura da faixa de abas (só aparece com 2 abas ou mais).
const ABAS: f32 = 30.0;
const MAX_ABAS: usize = 12;
const LATERAL: f32 = 200.0;
const RODAPE: f32 = 28.0;
const CEL_L: f32 = 112.0;
const CEL_A: f32 = 118.0;
const ICONE_G: f32 = 64.0;
const LINHA: f32 = 26.0;
const CABECALHO: f32 = 28.0;
const ICONE_P: f32 = 18.0;
const BORDA: f32 = 6.0;
const MAX_BITMAPS: usize = 900;

#[derive(Clone, Copy, PartialEq)]
enum Modo {
    Icones,
    Lista,
    /// Colunas como no Finder do Mac: cada pasta do caminho numa coluna.
    Colunas,
}

/// Largura de cada coluna no modo Colunas.
const COL_W: f32 = 230.0;

#[derive(Clone, Copy, PartialEq)]
enum Coluna {
    Nome,
    Data,
    Tamanho,
    Tipo,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Alvo {
    Fechar,
    Minimizar,
    Maximizar,
    Voltar,
    Avancar,
    ModoIcones,
    ModoLista,
    ModoColunas,
    /// Item `idx` da coluna `j` (pastas acima da atual, no modo Colunas).
    ColunaItem(usize, usize),
    Busca,
    Lugar(usize),
    Item(usize),
    Cabecalho(usize),
    Caminho(usize),
    Aba(usize),
    FecharAba(usize),
    NovaAba,
}

/// Uma aba guardada: o suficiente para voltar a ela como estava.
#[derive(Clone)]
struct Aba {
    pasta: PathBuf,
    historico: Vec<PathBuf>,
    pos: usize,
    modo: Modo,
    ordem: (Coluna, bool),
    rolagem: f32,
    selecao: Vec<PathBuf>,
}

impl Aba {
    fn nova(pasta: PathBuf, modo: Modo, ordem: (Coluna, bool)) -> Self {
        Self { historico: vec![pasta.clone()], pasta, pos: 0, modo, ordem, rolagem: 0.0, selecao: Vec::new() }
    }

    fn nome(&self) -> String {
        self.pasta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.pasta.to_string_lossy().into_owned())
    }
}

struct Previa {
    item: usize,
    chave: String,
    texto: Option<Vec<String>>,
}

pub struct Finder {
    sup: Superficie,
    vidro: Vidro,
    pincel: ID2D1SolidColorBrush,
    titulo: IDWriteTextFormat,
    texto: IDWriteTextFormat,
    pequeno: IDWriteTextFormat,
    rotulo: IDWriteTextFormat,
    secao: IDWriteTextFormat,
    mono: IDWriteTextFormat,
    centro: IDWriteTextFormat,
    /// Todas as abas; a de índice `aba` é a que está na tela (os campos
    /// abaixo valem para ela, e a cópia guardada só é atualizada ao trocar).
    abas: Vec<Aba>,
    aba: usize,
    /// Onde o botão foi apertado sobre um item (arrastar começa ao mexer).
    arrasto: Option<(f32, f32)>,
    /// Clique num item de uma seleção múltipla: só vira seleção única ao
    /// soltar (assim dá para arrastar todos).
    adiar: Option<usize>,
    mapa: Rc<RefCell<MapaSoltar>>,
    pasta: PathBuf,
    historico: Vec<PathBuf>,
    pos: usize,
    itens: Vec<Entrada>,
    /// Índices de `itens` na ordem em que aparecem (filtro e ordenação).
    visiveis: Vec<usize>,
    selecao: HashSet<usize>,
    ancora: Option<usize>,
    modo: Modo,
    ordem: (Coluna, bool),
    rolagem: Mola,
    busca: Campo,
    busca_ativa: bool,
    sub: Option<(Arc<AtomicBool>, Arc<Mutex<Vec<Entrada>>>)>,
    renomeando: Option<(usize, Campo)>,
    favoritos: Vec<(String, PathBuf)>,
    discos: Vec<(String, PathBuf)>,
    servico: ServicoIcones,
    bitmaps: HashMap<String, Option<ID2D1Bitmap1>>,
    _vigia: Option<Vigia>,
    zonas: Vec<(D2D_RECT_F, Alvo)>,
    hover: Option<Alvo>,
    ultimo_clique: Option<(Instant, Alvo)>,
    digitado: (String, Option<Instant>),
    previa: Option<Previa>,
    maximizada: Option<RECT>,
    larg: f32,
    alt: f32,
    erro: Option<String>,
    /// Modo Colunas: as pastas acima da atual, cada uma com o conteúdo e o
    /// item que está no caminho (destacado).
    ancestrais: Vec<(PathBuf, Vec<Entrada>, Option<PathBuf>)>,
    /// Pasta criada pela extração de um .zip (a thread preenche).
    zip: Arc<Mutex<Option<PathBuf>>>,
    pub fechar: bool,
    pub nova_janela: bool,
    pub sujo: bool,
    /// Está minimizada, e se isso acabou de mudar (a App leva ao Palco).
    pub minimizada: bool,
    pub mudou_minimizada: bool,
}

impl Drop for Finder {
    fn drop(&mut self) {
        if let Some((c, _)) = &self.sub {
            c.store(true, Ordering::Relaxed);
        }
        Soltar::revogar(self.sup.hwnd);
        unsafe {
            let _ = DestroyWindow(self.sup.hwnd);
        }
    }
}

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

fn dentro(r: &D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

impl Finder {
    pub fn abrir(gpu: &Gpu, pasta: PathBuf) -> Result<Self> {
        Self::abrir_com(gpu, pasta, true)
    }

    /// tivar = falso só no modo de teste (não rouba o teclado).
    pub fn abrir_com(gpu: &Gpu, pasta: PathBuf, ativar: bool) -> Result<Self> {
        let hwnd = janela::criar(WS_EX_APPWINDOW)?;
        unsafe {
            // Borda de redimensionar (a moldura em si some no WM_NCCALCSIZE).
            let estilo = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            SetWindowLongW(hwnd, GWL_STYLE, (estilo | WS_THICKFRAME.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0) as i32);
        }
        janela::preparar_vidro(hwnd);
        let e = janela::escala(hwnd);
        let (_, trab) = janela::monitor_do_cursor();
        let (l, a) = ((1060.0 * e) as i32, (660.0 * e) as i32);
        let l = l.min(trab.right - trab.left - 40);
        let a = a.min(trab.bottom - trab.top - 40);
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                HWND_TOP,
                trab.left + (trab.right - trab.left - l) / 2,
                trab.top + (trab.bottom - trab.top - a) / 2,
                l,
                a,
                SWP_FRAMECHANGED | SWP_NOACTIVATE,
            );
        }
        let mut sup = Superficie::new(gpu, hwnd)?;
        let _ = sup.arredondar(gpu, 12.0);
        let vidro = Vidro::new(&gpu.comp, &sup.raiz, TINTA_PAINEL)?;
        unsafe {
            let pincel = gpu.d2d_dc.CreateSolidColorBrush(&crate::tema::tinta(1.0), None)?;
            let fonte = |familia: PCWSTR, peso, tam, quebra: bool, centro: bool| -> Result<IDWriteTextFormat> {
                let f = gpu.dwrite.CreateTextFormat(familia, None, peso, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, tam, crate::idioma::localidade())?;
                f.SetParagraphAlignment(if quebra { DWRITE_PARAGRAPH_ALIGNMENT_NEAR } else { DWRITE_PARAGRAPH_ALIGNMENT_CENTER })?;
                f.SetWordWrapping(if quebra { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP })?;
                if centro {
                    f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                }
                let sinal = gpu.dwrite.CreateEllipsisTrimmingSign(&f)?;
                f.SetTrimming(
                    &DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 },
                    &sinal,
                )?;
                Ok(f)
            };
            let ui = w!("Segoe UI Variable Text");
            let mut f = Self {
                sup,
                vidro,
                pincel,
                titulo: fonte(ui, DWRITE_FONT_WEIGHT_SEMI_BOLD, 15.0, false, false)?,
                texto: fonte(ui, DWRITE_FONT_WEIGHT_NORMAL, 13.0, false, false)?,
                pequeno: fonte(ui, DWRITE_FONT_WEIGHT_NORMAL, 12.0, false, false)?,
                rotulo: fonte(ui, DWRITE_FONT_WEIGHT_NORMAL, 12.0, true, true)?,
                secao: fonte(ui, DWRITE_FONT_WEIGHT_SEMI_BOLD, 11.0, false, false)?,
                mono: fonte(w!("Cascadia Mono"), DWRITE_FONT_WEIGHT_NORMAL, 12.0, false, false)?,
                centro: fonte(ui, DWRITE_FONT_WEIGHT_NORMAL, 12.0, false, true)?,
                abas: vec![Aba::nova(pasta.clone(), Modo::Icones, (Coluna::Nome, true))],
                aba: 0,
                arrasto: None,
                adiar: None,
                mapa: Rc::new(RefCell::new(MapaSoltar::default())),
                pasta: pasta.clone(),
                historico: vec![pasta.clone()],
                pos: 0,
                itens: Vec::new(),
                visiveis: Vec::new(),
                selecao: HashSet::new(),
                ancora: None,
                modo: Modo::Icones,
                ordem: (Coluna::Nome, true),
                rolagem: Mola::default(),
                busca: Campo::default(),
                busca_ativa: false,
                sub: None,
                renomeando: None,
                favoritos: arquivos::favoritos(),
                discos: arquivos::discos(),
                servico: ServicoIcones::new(hwnd, WM_FINDER_ICONES),
                bitmaps: HashMap::new(),
                _vigia: None,
                zonas: Vec::new(),
                hover: None,
                ultimo_clique: None,
                digitado: (String::new(), None),
                previa: None,
                maximizada: None,
                larg: 0.0,
                alt: 0.0,
                erro: None,
                ancestrais: Vec::new(),
                zip: Arc::new(Mutex::new(None)),
                fechar: false,
                nova_janela: false,
                minimizada: false,
                mudou_minimizada: false,
                sujo: true,
            };
            Soltar::registrar(hwnd, WM_FINDER_SOLTAR, f.mapa.clone());
            f.ajustar_tamanho(gpu);
            f.carregar(false);
            f.desenhar(gpu)?;
            if ativar {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            f.sup.animar_entrada(gpu, 0.0, 0.97);
            Ok(f)
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.sup.hwnd
    }

    pub fn trazer(&self) {
        unsafe {
            if IsIconic(self.sup.hwnd).as_bool() {
                let _ = ShowWindow(self.sup.hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(self.sup.hwnd);
        }
    }

    fn escala(&self) -> f32 {
        self.sup.dpi / 96.0
    }

    fn ajustar_tamanho(&mut self, gpu: &Gpu) {
        let (l, a) = janela::tamanho_cliente(self.sup.hwnd);
        let _ = self.sup.redimensionar(gpu);
        let e = self.escala();
        let _ = self.vidro.definir(0.0, 0.0, l as f32, a as f32, 12.0 * e);
        self.larg = l as f32 / e;
        self.alt = a as f32 / e;
        self.limitar_rolagem();
        self.sujo = true;
    }

    // ------------------------------------------------------------------
    // Navegação

    pub fn ir(&mut self, p: PathBuf) {
        if p.as_os_str() == arquivos::LIXEIRA {
            arquivos::abrir_lixeira();
            return;
        }
        if p == self.pasta && self.sub.is_none() {
            return;
        }
        self.historico.truncate(self.pos + 1);
        self.historico.push(p.clone());
        self.pos = self.historico.len() - 1;
        self.pasta = p;
        self.carregar(false);
    }

    fn voltar(&mut self) {
        if self.pos > 0 {
            self.pos -= 1;
            self.pasta = self.historico[self.pos].clone();
            self.carregar(false);
        }
    }

    fn avancar(&mut self) {
        if self.pos + 1 < self.historico.len() {
            self.pos += 1;
            self.pasta = self.historico[self.pos].clone();
            self.carregar(false);
        }
    }

    fn subir(&mut self) {
        // A raiz de um disco sobe para "Este Computador".
        let pai = self.pasta.parent().map(|p| p.to_path_buf()).or_else(|| {
            (!arquivos::eh_computador(&self.pasta)).then(|| PathBuf::from(arquivos::COMPUTADOR))
        });
        if let Some(pai) = pai {
            let de = self.pasta.clone();
            self.ir(pai);
            // Seleciona a pasta de onde viemos.
            if let Some(k) = self.visiveis.iter().position(|&i| self.itens[i].caminho == de) {
                self.selecao = HashSet::from([self.visiveis[k]]);
                self.ancora = Some(self.visiveis[k]);
                self.mostrar_item(k);
            }
        }
    }

    // ------------------------------------------------------------------
    // Abas

    fn foto(&self) -> Aba {
        Aba {
            pasta: self.pasta.clone(),
            historico: self.historico.clone(),
            pos: self.pos,
            modo: self.modo,
            ordem: self.ordem,
            rolagem: self.rolagem.alvo,
            selecao: self.selecionados(),
        }
    }

    /// Põe a aba k na tela (sem guardar a atual: quem chama decide).
    fn carregar_aba(&mut self, k: usize) {
        self.aba = k;
        let a = self.abas[k].clone();
        self.pasta = a.pasta;
        self.historico = a.historico;
        self.pos = a.pos;
        self.modo = a.modo;
        self.ordem = a.ordem;
        self.carregar(false);
        self.selecao = self.itens.iter().enumerate().filter(|(_, e)| a.selecao.contains(&e.caminho)).map(|(i, _)| i).collect();
        self.rolagem = Mola::default();
        self.rolagem.x = a.rolagem;
        self.rolagem.alvo = a.rolagem;
        self.limitar_rolagem();
        self.sujo = true;
    }

    fn mostrar_aba(&mut self, k: usize) {
        if k == self.aba || k >= self.abas.len() {
            return;
        }
        self.terminar_renomear(true);
        self.abas[self.aba] = self.foto();
        self.carregar_aba(k);
    }

    /// Abre `pasta` numa aba nova logo depois da atual. `ir` = já mostrar.
    fn nova_aba(&mut self, pasta: PathBuf, ir: bool) {
        if self.abas.len() >= MAX_ABAS {
            if ir {
                self.ir(pasta);
            }
            return;
        }
        let nova = Aba::nova(pasta, self.modo, self.ordem);
        self.abas[self.aba] = self.foto();
        self.abas.insert(self.aba + 1, nova);
        if ir {
            self.carregar_aba(self.aba + 1);
        }
        self.ajustar_abas();
    }

    fn fechar_aba(&mut self, k: usize) {
        if self.abas.len() <= 1 {
            self.fechar = true;
            return;
        }
        self.abas.remove(k);
        if k == self.aba {
            self.carregar_aba(k.min(self.abas.len() - 1));
        } else if k < self.aba {
            self.aba -= 1;
        }
        self.ajustar_abas();
    }

    fn proxima_aba(&mut self, passo: isize) {
        let n = self.abas.len() as isize;
        if n > 1 {
            self.mostrar_aba(((self.aba as isize + passo).rem_euclid(n)) as usize);
        }
    }

    /// A faixa de abas aparece ou some: a área de conteúdo muda.
    fn ajustar_abas(&mut self) {
        self.limitar_rolagem();
        self.sujo = true;
    }

    /// Onde o conteúdo começa (abaixo da barra de ferramentas e das abas).
    fn topo(&self) -> f32 {
        TOPO + if self.abas.len() > 1 { ABAS } else { 0.0 }
    }

    /// Lê a pasta. `manter` = recarregar mantendo seleção e rolagem.
    fn carregar(&mut self, manter: bool) {
        if let Some((c, _)) = self.sub.take() {
            c.store(true, Ordering::Relaxed);
        }
        let selecionados: HashSet<PathBuf> = if manter {
            self.selecao.iter().filter_map(|&i| self.itens.get(i).map(|e| e.caminho.clone())).collect()
        } else {
            HashSet::new()
        };
        match arquivos::listar(&self.pasta) {
            Ok(v) => {
                self.itens = v;
                self.erro = None;
            }
            Err(e) => {
                self.itens.clear();
                self.erro = Some(if e.kind() == std::io::ErrorKind::PermissionDenied {
                    crate::idioma::t("Você não tem permissão para ver esta pasta.", "You don't have permission to see this folder.").into()
                } else {
                    crate::idioma::t("Não foi possível abrir esta pasta.", "This folder couldn't be opened.").into()
                });
            }
        }
        if !manter {
            self.busca.limpar();
            self.busca_ativa = false;
            self.rolagem = Mola::default();
            self.renomeando = None;
            self.previa = None;
            self._vigia = arquivos::vigiar(&self.pasta, self.sup.hwnd, WM_FINDER_PASTA);
            let nome: Vec<u16> = self.nome_da_pasta().encode_utf16().chain(Some(0)).collect();
            unsafe {
                let _ = SetWindowTextW(self.sup.hwnd, PCWSTR(nome.as_ptr()));
            }
        }
        self.selecao = self.itens.iter().enumerate().filter(|(_, e)| selecionados.contains(&e.caminho)).map(|(i, _)| i).collect();
        if !manter || self.modo == Modo::Colunas {
            self.atualizar_ancestrais();
        }
        self.refiltrar();
    }

    /// Modo Colunas: lê as pastas acima da atual.
    fn atualizar_ancestrais(&mut self) {
        self.ancestrais.clear();
        if self.modo != Modo::Colunas || self.sub.is_some() {
            return;
        }
        let cadeia: Vec<PathBuf> = self.segmentos_caminho().into_iter().map(|s| s.1).collect();
        // Só as que cabem na tela (no máximo 6, para não ler pastas à toa).
        let inicio = cadeia.len().saturating_sub(7);
        for j in inicio..cadeia.len().saturating_sub(1) {
            let itens = arquivos::listar(&cadeia[j]).unwrap_or_default();
            self.ancestrais.push((cadeia[j].clone(), itens, cadeia.get(j + 1).cloned()));
        }
    }

    /// Quantas colunas de pastas acima cabem antes da coluna atual.
    fn ancestrais_visiveis(&self) -> usize {
        let a = self.area();
        let cabem = (((a.right - a.left) / COL_W).floor() as usize).saturating_sub(1).max(0);
        self.ancestrais.len().min(cabem)
    }

    /// Onde começa a coluna da pasta atual (modo Colunas).
    fn x_coluna_atual(&self) -> f32 {
        self.area().left + self.ancestrais_visiveis() as f32 * COL_W
    }

    fn nome_da_pasta(&self) -> String {
        if self.sub.is_some() {
            return format!("{} \u{201c}{}\u{201d}", crate::idioma::t("Buscando", "Searching"), self.busca.string());
        }
        if arquivos::eh_computador(&self.pasta) {
            return crate::idioma::t("Este Computador", "This PC").into();
        }
        self.pasta
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.pasta.to_string_lossy().into_owned())
    }

    /// Aplica filtro (texto da busca) e ordenação.
    fn refiltrar(&mut self) {
        let termo = crate::catalogo::normalizar(&self.busca.string());
        let mut v: Vec<usize> = (0..self.itens.len())
            .filter(|&i| self.sub.is_some() || termo.is_empty() || crate::catalogo::normalizar(&self.itens[i].nome).contains(&termo))
            .collect();
        let (col, asc) = self.ordem;
        let it = &self.itens;
        v.sort_by(|&a, &b| {
            let (x, y) = (&it[a], &it[b]);
            let base = match col {
                Coluna::Nome => x.nome.to_lowercase().cmp(&y.nome.to_lowercase()),
                Coluna::Data => x.modificado.cmp(&y.modificado),
                Coluna::Tamanho => x.tamanho.cmp(&y.tamanho),
                Coluna::Tipo => x.tipo.cmp(&y.tipo).then_with(|| x.nome.to_lowercase().cmp(&y.nome.to_lowercase())),
            };
            y.pasta.cmp(&x.pasta).then(if asc { base } else { base.reverse() })
        });
        self.visiveis = v;
        self.limitar_rolagem();
        self.sujo = true;
    }

    /// Busca em subpastas (Enter no campo de busca).
    fn buscar_subpastas(&mut self) {
        let termo = self.busca.string();
        if termo.trim().is_empty() {
            return;
        }
        if let Some((c, _)) = self.sub.take() {
            c.store(true, Ordering::Relaxed);
        }
        let cancelar = Arc::new(AtomicBool::new(false));
        let saida = Arc::new(Mutex::new(Vec::new()));
        arquivos::buscar(self.pasta.clone(), termo, self.sup.hwnd, WM_FINDER_BUSCA, cancelar.clone(), saida.clone());
        self.sub = Some((cancelar, saida));
        self.itens.clear();
        self.selecao.clear();
        self.rolagem = Mola::default();
        self.modo = Modo::Lista;
        self.refiltrar();
    }

    /// Busca vinda de fora (Alt+Espaço > "Buscar arquivos").
    pub fn buscar_por(&mut self, termo: &str) {
        self.busca.limpar();
        self.busca.inserir(termo);
        self.buscar_subpastas();
    }

    fn abrir_item(&mut self, k: usize) {
        let Some(&i) = self.visiveis.get(k) else { return };
        let e = self.itens[i].clone();
        if e.pasta {
            self.ir(e.caminho);
        } else if arquivos::eh_zip(&e.caminho) {
            // Como no Mac: extrai ao lado e abre a pasta.
            arquivos::extrair_zip(e.caminho, self.sup.hwnd, WM_FINDER_ZIP, self.zip.clone());
        } else {
            arquivos::abrir(&e.caminho);
        }
    }

    fn selecionados(&self) -> Vec<PathBuf> {
        self.visiveis.iter().filter(|i| self.selecao.contains(i)).map(|&i| self.itens[i].caminho.clone()).collect()
    }

    fn foco(&self) -> Option<usize> {
        self.ancora.and_then(|a| self.visiveis.iter().position(|&i| i == a))
    }

    fn selecionar(&mut self, k: usize, ctrl: bool, shift: bool) {
        let Some(&i) = self.visiveis.get(k) else { return };
        if shift {
            let de = self.foco().unwrap_or(k);
            let (a, b) = (de.min(k), de.max(k));
            if !ctrl {
                self.selecao.clear();
            }
            for j in a..=b {
                self.selecao.insert(self.visiveis[j]);
            }
            return;
        }
        if ctrl {
            if !self.selecao.remove(&i) {
                self.selecao.insert(i);
            }
        } else {
            self.selecao = HashSet::from([i]);
        }
        self.ancora = Some(i);
    }

    // ------------------------------------------------------------------
    // Geometria (DIPs)

    fn area(&self) -> D2D_RECT_F {
        retangulo(LATERAL, self.topo(), self.larg, self.alt - RODAPE)
    }

    fn colunas(&self) -> usize {
        (((self.larg - LATERAL - 24.0) / CEL_L).floor() as usize).max(1)
    }

    fn altura_conteudo(&self) -> f32 {
        let n = self.visiveis.len();
        match self.modo {
            Modo::Icones => (n.div_ceil(self.colunas())) as f32 * CEL_A + 24.0,
            Modo::Lista => CABECALHO + n as f32 * LINHA + 8.0,
            Modo::Colunas => n as f32 * LINHA + 12.0,
        }
    }

    fn limitar_rolagem(&mut self) {
        let a = self.area();
        let max = (self.altura_conteudo() - (a.bottom - a.top)).max(0.0);
        self.rolagem.alvo = self.rolagem.alvo.clamp(0.0, max);
        if self.rolagem.x > max {
            self.rolagem.x = max;
        }
    }

    /// Retângulo do item k (já com a rolagem).
    fn ret_item(&self, k: usize) -> D2D_RECT_F {
        let a = self.area();
        match self.modo {
            Modo::Icones => {
                let c = self.colunas();
                let x = a.left + 12.0 + (k % c) as f32 * CEL_L;
                let y = a.top + 12.0 + (k / c) as f32 * CEL_A - self.rolagem.x;
                retangulo(x, y, x + CEL_L, y + CEL_A)
            }
            Modo::Lista => {
                let y = a.top + CABECALHO + k as f32 * LINHA - self.rolagem.x;
                retangulo(a.left + 8.0, y, a.right - 8.0, y + LINHA)
            }
            Modo::Colunas => {
                let x0 = self.x_coluna_atual();
                let y = a.top + 6.0 + k as f32 * LINHA - self.rolagem.x;
                retangulo(x0 + 5.0, y, x0 + COL_W - 5.0, y + LINHA)
            }
        }
    }

    fn mostrar_item(&mut self, k: usize) {
        let a = self.area();
        let r = self.ret_item(k);
        let topo = if self.modo == Modo::Lista { a.top + CABECALHO } else { a.top };
        if r.top < topo {
            self.rolagem.alvo -= topo - r.top;
        } else if r.bottom > a.bottom {
            self.rolagem.alvo += r.bottom - a.bottom;
        }
        self.limitar_rolagem();
        self.sujo = true;
    }

    fn faixa_visivel(&self) -> std::ops::Range<usize> {
        let a = self.area();
        let n = self.visiveis.len();
        let rol = self.rolagem.x;
        match self.modo {
            Modo::Icones => {
                let c = self.colunas();
                let l0 = ((rol - 12.0) / CEL_A).floor().max(0.0) as usize;
                let l1 = ((rol + (a.bottom - a.top)) / CEL_A).ceil() as usize + 1;
                (l0 * c).min(n)..(l1 * c).min(n)
            }
            Modo::Lista | Modo::Colunas => {
                let k0 = (rol / LINHA).floor().max(0.0) as usize;
                let k1 = ((rol + (a.bottom - a.top)) / LINHA).ceil() as usize + 1;
                k0.min(n)..k1.min(n)
            }
        }
    }

    fn colunas_lista(&self) -> [(f32, f32, &'static str, Coluna); 4] {
        let a = self.area();
        let l = a.right - a.left - 16.0;
        let x0 = a.left + 8.0;
        let nome = l * 0.46;
        let data = l * 0.22;
        let tam = l * 0.12;
        [
            (x0, x0 + nome, crate::idioma::t("Nome", "Name"), Coluna::Nome),
            (x0 + nome, x0 + nome + data, crate::idioma::t("Modificado", "Modified"), Coluna::Data),
            (x0 + nome + data, x0 + nome + data + tam, crate::idioma::t("Tamanho", "Size"), Coluna::Tamanho),
            (x0 + nome + data + tam, a.right - 8.0, crate::idioma::t("Tipo", "Kind"), Coluna::Tipo),
        ]
    }

    fn segmentos_caminho(&self) -> Vec<(String, PathBuf)> {
        let mut v = Vec::new();
        let mut p = (!arquivos::eh_computador(&self.pasta)).then_some(self.pasta.as_path());
        while let Some(atual) = p {
            let nome = atual.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| atual.to_string_lossy().trim_end_matches('\\').to_string());
            v.push((nome, atual.to_path_buf()));
            p = atual.parent();
        }
        v.push((crate::idioma::t("Este Computador", "This PC").into(), PathBuf::from(arquivos::COMPUTADOR)));
        v.reverse();
        v
    }

    fn acertar(&self, x: f32, y: f32) -> Option<Alvo> {
        self.zonas.iter().rev().find(|(r, _)| dentro(r, x, y)).map(|z| z.1)
    }

    // ------------------------------------------------------------------
    // Mensagens

    pub fn animando(&self) -> bool {
        !self.rolagem.parada()
    }

    pub fn passo(&mut self, dt: f32) {
        if self.animando() {
            self.rolagem.passo(dt);
            self.sujo = true;
        }
    }

    pub fn mensagem(&mut self, gpu: &Gpu, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let e = self.escala();
        let x = (lp.0 & 0xffff) as i16 as f32 / e;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / e;
        match msg {
            WM_NCCALCSIZE if wp.0 != 0 => Some(LRESULT(0)), // sem moldura do Windows
            WM_NCHITTEST => Some(LRESULT(self.hit_test(lp))),
            WM_GETMINMAXINFO => {
                let mm = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
                mm.ptMinTrackSize = POINT { x: (680.0 * e) as i32, y: (420.0 * e) as i32 };
                Some(LRESULT(0))
            }
            WM_SIZE => {
                // Minimizar e restaurar: o Windows não avisa o Cais sobre as
                // janelas dele mesmo, então a própria janela avisa (para ganhar
                // a telinha no Palco, como as outras).
                let minimizada = wp.0 as u32 == SIZE_MINIMIZED;
                if minimizada != self.minimizada {
                    self.minimizada = minimizada;
                    self.mudou_minimizada = true;
                }
                if !minimizada {
                    self.ajustar_tamanho(gpu);
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let r = unsafe { &*(lp.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(self.sup.hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                self.ajustar_tamanho(gpu);
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                self.sujo = true;
                None
            }
            WM_TIMER if wp.0 == TIMER_RECARREGAR => {
                unsafe {
                    let _ = KillTimer(self.sup.hwnd, TIMER_RECARREGAR);
                }
                if self.sub.is_none() {
                    self.carregar(true);
                }
                Some(LRESULT(0))
            }
            WM_FINDER_PASTA => {
                // Junta vários avisos seguidos (cópia de muitos arquivos).
                unsafe {
                    SetTimer(self.sup.hwnd, TIMER_RECARREGAR, 250, None);
                }
                Some(LRESULT(0))
            }
            WM_FINDER_ICONES => {
                for (chave, p) in self.servico.receber() {
                    let b = p.and_then(|p| crate::apps::bitmap_de_pixels(&gpu.d2d_dc, &p));
                    self.bitmaps.insert(chave, b);
                }
                self.sujo = true;
                Some(LRESULT(0))
            }
            WM_FINDER_BUSCA => {
                if let Some((_, s)) = &self.sub {
                    let novos = s.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
                    self.itens.extend(novos);
                    self.refiltrar();
                }
                Some(LRESULT(0))
            }
            WM_FINDER_SOLTAR => {
                self.sujo = true;
                Some(LRESULT(0))
            }
            WM_FINDER_ZIP => {
                let pasta = self.zip.lock().ok().and_then(|mut g| g.take());
                match pasta {
                    Some(p) if wp.0 != 0 => self.ir(p),
                    _ => unsafe {
                        MessageBoxW(
                            self.sup.hwnd,
                            PCWSTR(crate::idioma::largo(crate::idioma::t("Não foi possível extrair este arquivo .zip.", "This .zip file couldn't be extracted.")).as_ptr()),
                            PCWSTR(crate::idioma::largo(crate::idioma::t("Arquivos", "Files")).as_ptr()),
                            MB_OK | MB_ICONWARNING,
                        );
                    },
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                // Botão apertado sobre um item e o mouse andou: arrastar.
                if wp.0 & 0x0001 != 0 {
                    if let Some((x0, y0)) = self.arrasto {
                        if (x - x0).abs() > 5.0 || (y - y0).abs() > 5.0 {
                            self.arrasto = None;
                            self.adiar = None;
                            self.arrastar();
                            return Some(LRESULT(0));
                        }
                    }
                } else {
                    self.arrasto = None;
                }
                let h = self.acertar(x, y);
                if h != self.hover {
                    self.hover = h;
                    self.sujo = true;
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let d = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                self.rolagem.alvo -= d / 120.0 * 90.0;
                self.limitar_rolagem();
                self.sujo = true;
                Some(LRESULT(0))
            }
            WM_XBUTTONUP => {
                if (wp.0 >> 16) & 0xffff == 1 {
                    self.voltar();
                } else {
                    self.avancar();
                }
                Some(LRESULT(1))
            }
            WM_LBUTTONDOWN => {
                self.clique(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                self.arrasto = None;
                if let Some(k) = self.adiar.take() {
                    self.selecionar(k, false, false);
                    self.sujo = true;
                }
                Some(LRESULT(0))
            }
            WM_MBUTTONUP => {
                // Botão do meio: pasta abre numa aba nova; numa aba, fecha.
                match self.acertar(x, y) {
                    Some(Alvo::Item(k)) => {
                        if let Some(e) = self.visiveis.get(k).map(|&i| self.itens[i].clone()) {
                            if e.pasta {
                                self.nova_aba(e.caminho, false);
                            }
                        }
                    }
                    Some(Alvo::Aba(k)) | Some(Alvo::FecharAba(k)) => self.fechar_aba(k),
                    Some(Alvo::Lugar(i)) => {
                        let lugares: Vec<PathBuf> = self.favoritos.iter().chain(self.discos.iter()).map(|l| l.1.clone()).collect();
                        if let Some(p) = lugares.get(i) {
                            self.nova_aba(p.clone(), false);
                        }
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                self.clique_direito(x, y);
                Some(LRESULT(0))
            }
            WM_CHAR => {
                self.caractere(wp.0 as u32);
                Some(LRESULT(0))
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => self.tecla(gpu, msg, wp),
            _ => None,
        }
    }

    fn hit_test(&self, lp: LPARAM) -> isize {
        let e = self.escala();
        let mut p = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
        unsafe {
            let _ = ScreenToClient(self.sup.hwnd, &mut p);
        }
        let (x, y) = (p.x as f32 / e, p.y as f32 / e);
        if self.maximizada.is_none() {
            let (esq, dir, cima, baixo) = (x < BORDA, x > self.larg - BORDA, y < BORDA, y > self.alt - BORDA);
            let v = match (esq, dir, cima, baixo) {
                (true, _, true, _) => HTTOPLEFT,
                (_, true, true, _) => HTTOPRIGHT,
                (true, _, _, true) => HTBOTTOMLEFT,
                (_, true, _, true) => HTBOTTOMRIGHT,
                (true, _, _, _) => HTLEFT,
                (_, true, _, _) => HTRIGHT,
                (_, _, true, _) => HTTOP,
                (_, _, _, true) => HTBOTTOM,
                _ => 0,
            };
            if v != 0 {
                return v as isize;
            }
        }
        if y < TOPO && self.acertar(x, y).is_none() {
            return HTCAPTION as isize;
        }
        HTCLIENT as isize
    }

    fn clique(&mut self, x: f32, y: f32) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let alvo = self.acertar(x, y);
        let duplo = alvo.is_some()
            && self.ultimo_clique.is_some_and(|(t, a)| Some(a) == alvo && t.elapsed().as_millis() < unsafe { GetDoubleClickTime() } as u128);
        self.ultimo_clique = alvo.map(|a| (Instant::now(), a));
        if self.renomeando.is_some() && !matches!(alvo, Some(Alvo::Item(_))) {
            self.terminar_renomear(true);
        }
        self.busca_ativa = alvo == Some(Alvo::Busca);
        if let Some(p) = &self.previa {
            let _ = p;
            self.previa = None;
        }
        match alvo {
            Some(Alvo::Fechar) => self.fechar = true,
            Some(Alvo::Minimizar) => unsafe {
                let _ = ShowWindow(self.sup.hwnd, SW_MINIMIZE);
            },
            Some(Alvo::Maximizar) => self.alternar_maximizar(),
            Some(Alvo::Voltar) => self.voltar(),
            Some(Alvo::Avancar) => self.avancar(),
            Some(Alvo::ModoIcones) => self.trocar_modo(Modo::Icones),
            Some(Alvo::ModoLista) => self.trocar_modo(Modo::Lista),
            Some(Alvo::ModoColunas) => self.trocar_modo(Modo::Colunas),
            Some(Alvo::ColunaItem(j, idx)) => {
                if let Some(e) = self.ancestrais.get(j).and_then(|a| a.1.get(idx)).cloned() {
                    if e.pasta {
                        self.ir(e.caminho);
                    } else if let Some(pai) = e.caminho.parent().map(|p| p.to_path_buf()) {
                        self.ir(pai);
                        if let Some(k) = self.visiveis.iter().position(|&i| self.itens[i].caminho == e.caminho) {
                            self.selecionar(k, false, false);
                            self.mostrar_item(k);
                        }
                    }
                }
            }
            Some(Alvo::Busca) => {}
            Some(Alvo::Lugar(i)) => {
                let lugares: Vec<PathBuf> = self.favoritos.iter().chain(self.discos.iter()).map(|l| l.1.clone()).collect();
                if let Some(p) = lugares.get(i) {
                    self.ir(p.clone());
                }
            }
            Some(Alvo::Cabecalho(c)) => {
                let col = self.colunas_lista()[c].3;
                self.ordem = if self.ordem.0 == col { (col, !self.ordem.1) } else { (col, true) };
                self.refiltrar();
            }
            Some(Alvo::Caminho(i)) => {
                if let Some((_, p)) = self.segmentos_caminho().get(i) {
                    self.ir(p.clone());
                }
            }
            Some(Alvo::Aba(k)) => self.mostrar_aba(k),
            Some(Alvo::FecharAba(k)) => self.fechar_aba(k),
            Some(Alvo::NovaAba) => self.nova_aba(self.pasta.clone(), true),
            Some(Alvo::Item(k)) => {
                let i = self.visiveis.get(k).copied();
                let pasta = i.filter(|&i| self.itens[i].pasta).map(|i| self.itens[i].caminho.clone());
                if self.modo == Modo::Colunas && !ctrl && !shift && pasta.is_some() {
                    // Colunas: um clique numa pasta já mostra o conteúdo dela.
                    self.ir(pasta.unwrap());
                } else if duplo && ctrl && !shift && pasta.is_some() {
                    // Ctrl + duplo clique numa pasta: abre numa aba nova.
                    self.nova_aba(pasta.unwrap(), true);
                } else if duplo && !ctrl && !shift {
                    self.abrir_item(k);
                } else if !ctrl && !shift && i.is_some_and(|i| self.selecao.contains(&i)) && self.selecao.len() > 1 {
                    self.adiar = Some(k);
                    self.ancora = i;
                } else {
                    self.selecionar(k, ctrl, shift);
                }
                if !duplo {
                    self.arrasto = Some((x, y));
                }
            }
            None => {
                if dentro(&self.area(), x, y) && !ctrl {
                    self.selecao.clear();
                }
            }
        }
        self.sujo = true;
    }

    /// Arrasta os selecionados para fora (ou para uma pasta aqui dentro).
    fn arrastar(&mut self) {
        let mut itens = self.selecionados();
        let Some(pai) = itens.first().and_then(|p| p.parent()).map(|p| p.to_path_buf()) else { return };
        // Resultado de busca mistura pastas: vão juntos só os da mesma pasta.
        itens.retain(|p| p.parent() == Some(pai.as_path()));
        arquivos::arrastar(self.sup.hwnd, &itens, &pai);
        self.hover = None;
        self.sujo = true;
    }

    fn clique_direito(&mut self, x: f32, y: f32) {
        self.terminar_renomear(true);
        let alvo = self.acertar(x, y);
        if let Some(Alvo::Item(k)) = alvo {
            if let Some(&i) = self.visiveis.get(k) {
                if !self.selecao.contains(&i) {
                    self.selecao = HashSet::from([i]);
                    self.ancora = Some(i);
                }
            }
        } else if dentro(&self.area(), x, y) {
            self.selecao.clear();
        } else {
            return;
        }
        self.sujo = true;
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        let itens = self.selecionados();
        match arquivos::menu_contexto(self.sup.hwnd, &itens, &self.pasta, p.x, p.y) {
            Escolha::Renomear => self.comecar_renomear(),
            Escolha::NovaAba => {
                if let Some(pasta) = itens.first().cloned() {
                    self.nova_aba(pasta, true);
                }
            }
            Escolha::NoExplorador => {
                if let Some(pasta) = itens.first() {
                    crate::pastas::deixar_passar();
                    arquivos::abrir_no_explorador(pasta);
                }
            }
            _ => {}
        }
    }

    fn trocar_modo(&mut self, m: Modo) {
        if self.modo != m {
            self.modo = m;
            self.atualizar_ancestrais();
            self.rolagem = Mola::default();
            if let Some(k) = self.foco() {
                self.mostrar_item(k);
            }
            self.sujo = true;
        }
    }

    fn alternar_maximizar(&mut self) {
        unsafe {
            match self.maximizada.take() {
                Some(r) => {
                    let _ = SetWindowPos(self.sup.hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                None => {
                    let mut r = RECT::default();
                    let _ = GetWindowRect(self.sup.hwnd, &mut r);
                    self.maximizada = Some(r);
                    let (_, t) = janela::monitor_do_cursor();
                    let _ = SetWindowPos(self.sup.hwnd, None, t.left, t.top, t.right - t.left, t.bottom - t.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        }
    }

    fn caractere(&mut self, c: u32) {
        if let Some((_, campo)) = &mut self.renomeando {
            campo.caractere(c);
            self.sujo = true;
            return;
        }
        if self.busca_ativa {
            if let Tecla::Mudou = self.busca.caractere(c) {
                if self.sub.is_some() {
                    self.carregar(false);
                    self.busca_ativa = true;
                }
                self.refiltrar();
            }
            return;
        }
        // Digitar um nome pula para o primeiro item que começa assim.
        let Some(ch) = char::from_u32(c).filter(|c| !c.is_control() && *c != ' ') else { return };
        let novo = self.digitado.1.map_or(true, |t| t.elapsed().as_millis() > 900);
        if novo {
            self.digitado.0.clear();
        }
        self.digitado.0.push(ch);
        self.digitado.1 = Some(Instant::now());
        let prefixo = crate::catalogo::normalizar(&self.digitado.0);
        if let Some(k) = self.visiveis.iter().position(|&i| crate::catalogo::normalizar(&self.itens[i].nome).starts_with(&prefixo)) {
            self.selecionar(k, false, false);
            self.mostrar_item(k);
        }
    }

    fn tecla(&mut self, gpu: &Gpu, msg: u32, wp: WPARAM) -> Option<LRESULT> {
        let vk = VIRTUAL_KEY(wp.0 as u16);
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let alt = msg == WM_SYSKEYDOWN;
        self.sujo = true;
        // Renomeando: o campo recebe tudo.
        if self.renomeando.is_some() {
            match vk {
                VK_RETURN => self.terminar_renomear(true),
                VK_ESCAPE => self.terminar_renomear(false),
                _ => {
                    if let Some((_, c)) = &mut self.renomeando {
                        c.tecla(vk.0);
                    }
                }
            }
            return Some(LRESULT(0));
        }
        if self.busca_ativa {
            match vk {
                VK_ESCAPE => {
                    self.busca_ativa = false;
                    if self.sub.is_some() {
                        self.carregar(false);
                    } else {
                        self.busca.limpar();
                        self.refiltrar();
                    }
                }
                VK_RETURN => self.buscar_subpastas(),
                VK_DOWN => self.busca_ativa = false,
                _ => {
                    if let Tecla::Mudou = self.busca.tecla(vk.0) {
                        self.refiltrar();
                    }
                }
            }
            return Some(LRESULT(0));
        }
        if self.previa.is_some() {
            match vk {
                VK_SPACE | VK_ESCAPE => {
                    self.previa = None;
                    return Some(LRESULT(0));
                }
                VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN => {}
                _ => return Some(LRESULT(0)),
            }
        }
        let n = self.visiveis.len();
        let passo = |modo: Modo, cols: usize| if modo == Modo::Icones { cols } else { 1 };
        let mover = |s: &mut Self, alvo: isize| {
            if n == 0 {
                return;
            }
            let k = alvo.clamp(0, n as isize - 1) as usize;
            s.selecionar(k, false, shift);
            if !shift {
                s.ancora = Some(s.visiveis[k]);
            }
            s.mostrar_item(k);
            if s.previa.is_some() {
                s.abrir_previa(gpu);
            }
        };
        let atual = self.foco().map(|k| k as isize).unwrap_or(-1);
        match vk {
            VK_LEFT if alt => self.voltar(),
            VK_RIGHT if alt => self.avancar(),
            VK_UP if alt => self.subir(),
            VK_LEFT if self.modo == Modo::Icones => mover(self, atual - 1),
            VK_RIGHT if self.modo == Modo::Icones => mover(self, atual + 1),
            VK_LEFT if self.modo == Modo::Colunas => self.subir(),
            VK_RIGHT if self.modo == Modo::Colunas => {
                if let Some(e) = self.foco().map(|k| self.itens[self.visiveis[k]].clone()).filter(|e| e.pasta) {
                    self.ir(e.caminho);
                    if !self.visiveis.is_empty() {
                        self.selecionar(0, false, false);
                    }
                }
            }
            VK_UP => mover(self, if atual < 0 { 0 } else { atual - passo(self.modo, self.colunas()) as isize }),
            VK_DOWN => mover(self, atual + passo(self.modo, self.colunas()) as isize),
            VK_HOME => mover(self, 0),
            VK_END => mover(self, n as isize - 1),
            VK_RETURN => {
                if let Some(k) = self.foco() {
                    self.abrir_item(k);
                }
            }
            VK_BACK => self.subir(),
            VK_SPACE => self.abrir_previa(gpu),
            VK_F2 => self.comecar_renomear(),
            VK_F5 => self.carregar(true),
            VK_DELETE => arquivos::verbo(self.sup.hwnd, &self.selecionados(), &self.pasta, "delete"),
            VK_ESCAPE => self.selecao.clear(),
            VIRTUAL_KEY(0x41) if ctrl => {
                // Ctrl+A
                self.selecao = self.visiveis.iter().copied().collect();
            }
            VIRTUAL_KEY(0x43) if ctrl => arquivos::verbo(self.sup.hwnd, &self.selecionados(), &self.pasta, "copy"),
            VIRTUAL_KEY(0x58) if ctrl => arquivos::verbo(self.sup.hwnd, &self.selecionados(), &self.pasta, "cut"),
            VIRTUAL_KEY(0x56) if ctrl => arquivos::verbo(self.sup.hwnd, &[], &self.pasta, "paste"),
            VIRTUAL_KEY(0x46) if ctrl => self.busca_ativa = true,
            VIRTUAL_KEY(0x57) if ctrl => self.fechar_aba(self.aba),
            VIRTUAL_KEY(0x54) if ctrl => self.nova_aba(self.pasta.clone(), true),
            VK_TAB if ctrl => self.proxima_aba(if shift { -1 } else { 1 }),
            VIRTUAL_KEY(0x4E) if ctrl && shift => {
                if let Some(p) = arquivos::nova_pasta(&self.pasta) {
                    self.carregar(true);
                    if let Some(k) = self.visiveis.iter().position(|&i| self.itens[i].caminho == p) {
                        self.selecionar(k, false, false);
                        self.mostrar_item(k);
                        self.comecar_renomear();
                    }
                }
            }
            VIRTUAL_KEY(0x4E) if ctrl => self.nova_janela = true,
            VIRTUAL_KEY(0x31) if ctrl => self.trocar_modo(Modo::Icones),
            VIRTUAL_KEY(0x32) if ctrl => self.trocar_modo(Modo::Lista),
            VIRTUAL_KEY(0x33) if ctrl => self.trocar_modo(Modo::Colunas),
            _ => return None,
        }
        Some(LRESULT(0))
    }

    // ------------------------------------------------------------------
    // Renomear e pré-visualizar

    fn comecar_renomear(&mut self) {
        let Some(k) = self.foco() else { return };
        let i = self.visiveis[k];
        let mut c = Campo::default();
        c.inserir(&self.itens[i].nome);
        // Seleciona só o nome (sem a extensão), como no Explorer.
        c.tudo = true;
        self.renomeando = Some((i, c));
        self.mostrar_item(k);
        self.sujo = true;
    }

    fn terminar_renomear(&mut self, salvar: bool) {
        let Some((i, c)) = self.renomeando.take() else { return };
        self.sujo = true;
        if !salvar {
            return;
        }
        let novo = c.string().trim().to_string();
        let Some(e) = self.itens.get(i) else { return };
        if novo.is_empty() || novo == e.nome || novo.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
            return;
        }
        let destino = e.caminho.with_file_name(&novo);
        if std::fs::rename(&e.caminho, &destino).is_ok() {
            self.carregar(true);
            if let Some(k) = self.visiveis.iter().position(|&j| self.itens[j].caminho == destino) {
                self.selecionar(k, false, false);
            }
        } else {
            unsafe {
                MessageBoxW(
                    self.sup.hwnd,
                    PCWSTR(crate::idioma::largo(crate::idioma::t("Não foi possível renomear (o arquivo pode estar em uso).", "Couldn't rename (the file may be in use).")).as_ptr()),
                    PCWSTR(crate::idioma::largo(crate::idioma::t("Arquivos", "Files")).as_ptr()),
                    MB_OK | MB_ICONWARNING,
                );
            }
        }
    }

    fn abrir_previa(&mut self, gpu: &Gpu) {
        let Some(k) = self.foco() else { return };
        let i = self.visiveis[k];
        let e = &self.itens[i];
        let chave = format!("previa:{}", e.caminho.to_string_lossy());
        let texto = if arquivos::eh_texto(&e.caminho) {
            std::fs::read(&e.caminho).ok().map(|b| {
                let s = String::from_utf8_lossy(&b[..b.len().min(64 * 1024)]).into_owned();
                s.lines().take(200).map(|l| l.replace('\t', "    ")).collect()
            })
        } else {
            None
        };
        if texto.is_none() && !self.bitmaps.contains_key(&chave) {
            let px = ((self.larg.min(self.alt) * 0.7) * self.escala()).round() as u32;
            self.servico.pedir(&chave, &e.caminho, px.clamp(256, 1600), arquivos::eh_imagem(&e.caminho) || e.miniatura);
        }
        let _ = gpu;
        self.previa = Some(Previa { item: i, chave, texto });
        self.sujo = true;
    }

    // ------------------------------------------------------------------
    // Desenho

    fn pedir_icones(&mut self) {
        let px = (96.0 * self.escala()).round() as u32;
        let faixa = self.faixa_visivel();
        let mut pedir = Vec::new();
        for k in faixa.clone() {
            let e = &self.itens[self.visiveis[k]];
            if !self.bitmaps.contains_key(&e.chave) {
                pedir.push((e.chave.clone(), e.caminho.clone(), e.miniatura));
            }
        }
        // Modo Colunas: ícones das pastas acima também.
        for (_, itens, _) in &self.ancestrais {
            for e in itens.iter().take(60) {
                if !self.bitmaps.contains_key(&e.chave) && !pedir.iter().any(|(c, ..)| *c == e.chave) {
                    pedir.push((e.chave.clone(), e.caminho.clone(), e.miniatura));
                }
            }
        }
        for (c, p, m) in pedir {
            self.servico.pedir(&c, &p, px, m);
        }
        // Memória: muitas miniaturas guardadas? Fica só o que está na tela
        // e os ícones genéricos por tipo.
        if self.bitmaps.len() > MAX_BITMAPS {
            let na_tela: HashSet<String> = faixa.map(|k| self.itens[self.visiveis[k]].chave.clone()).collect();
            let previa = self.previa.as_ref().map(|p| p.chave.clone());
            self.bitmaps.retain(|k, _| k.starts_with('.') || na_tela.contains(k) || Some(k) == previa.as_ref());
            for k in na_tela {
                self.servico.esquecer(&k);
            }
        }
    }

    pub fn desenhar(&mut self, gpu: &Gpu) -> Result<()> {
        self.pedir_icones();
        let dw = &gpu.dwrite;
        let lay = |s: &str, f: &IDWriteTextFormat, l: f32, a: f32| -> Result<IDWriteTextLayout> {
            let w: Vec<u16> = s.encode_utf16().collect();
            unsafe { dw.CreateTextLayout(&w, f, l.max(1.0), a.max(1.0)) }
        };
        let mut zonas: Vec<(D2D_RECT_F, Alvo)> = Vec::new();
        let ativa = unsafe { GetForegroundWindow() } == self.sup.hwnd;
        let (larg, alt) = (self.larg, self.alt);
        let area = self.area();

        // Botões da janela (fechar, minimizar, maximizar).
        let botoes = [(20.0, Alvo::Fechar, (1.0, 0.37, 0.34)), (40.0, Alvo::Minimizar, (1.0, 0.74, 0.18)), (60.0, Alvo::Maximizar, (0.16, 0.79, 0.25))];
        for (x, a, _) in botoes {
            zonas.push((retangulo(x - 8.0, 12.0, x + 8.0, 28.0), a));
        }

        // Barra lateral.
        let mut lateral: Vec<(IDWriteTextLayout, f32, Option<usize>)> = Vec::new(); // (texto, y, lugar)
        let mut y = 52.0;
        lateral.push((lay(crate::idioma::t("Favoritos", "Favorites"), &self.secao, LATERAL - 30.0, 20.0)?, y, None));
        y += 22.0;
        let mut k = 0;
        for (nome, _) in &self.favoritos {
            lateral.push((lay(nome, &self.texto, LATERAL - 52.0, 26.0)?, y, Some(k)));
            zonas.push((retangulo(10.0, y, LATERAL - 10.0, y + 26.0), Alvo::Lugar(k)));
            y += 28.0;
            k += 1;
        }
        y += 10.0;
        lateral.push((lay(crate::idioma::t("Locais", "Locations"), &self.secao, LATERAL - 30.0, 20.0)?, y, None));
        y += 22.0;
        for (nome, _) in &self.discos {
            lateral.push((lay(nome, &self.texto, LATERAL - 52.0, 26.0)?, y, Some(k)));
            zonas.push((retangulo(10.0, y, LATERAL - 10.0, y + 26.0), Alvo::Lugar(k)));
            y += 28.0;
            k += 1;
        }
        let lugares: Vec<PathBuf> = self.favoritos.iter().chain(self.discos.iter()).map(|l| l.1.clone()).collect();
        let lugar_atual = lugares.iter().position(|p| *p == self.pasta);
        let eh_disco = |k: usize| k >= self.favoritos.len();

        // Barra de ferramentas.
        let tx = LATERAL + 14.0;
        zonas.push((retangulo(tx, 12.0, tx + 28.0, 40.0), Alvo::Voltar));
        zonas.push((retangulo(tx + 30.0, 12.0, tx + 58.0, 40.0), Alvo::Avancar));
        let titulo = lay(&self.nome_da_pasta(), &self.titulo, (larg - tx - 70.0 - 330.0).max(80.0), TOPO)?;
        let busca_r = retangulo(larg - 16.0 - 200.0, 12.0, larg - 16.0, 40.0);
        zonas.push((busca_r, Alvo::Busca));
        let modo_i = retangulo(busca_r.left - 104.0, 12.0, busca_r.left - 74.0, 40.0);
        let modo_l = retangulo(busca_r.left - 74.0, 12.0, busca_r.left - 44.0, 40.0);
        let modo_c = retangulo(busca_r.left - 44.0, 12.0, busca_r.left - 14.0, 40.0);
        zonas.push((modo_i, Alvo::ModoIcones));
        zonas.push((modo_l, Alvo::ModoLista));
        zonas.push((modo_c, Alvo::ModoColunas));
        let busca_txt = if self.busca.texto.is_empty() { crate::idioma::t("Buscar", "Search").to_string() } else { self.busca.string() };
        let busca_lay = lay(&busca_txt, &self.texto, busca_r.right - busca_r.left - 34.0, 28.0)?;
        let mut cursor_busca = 0.0;
        if self.busca_ativa && !self.busca.texto.is_empty() {
            let pos16: usize = self.busca.texto[..self.busca.cursor].iter().map(|c| c.len_utf16()).sum();
            let (mut cx, mut cy, mut m) = (0.0f32, 0.0f32, DWRITE_HIT_TEST_METRICS::default());
            unsafe {
                let _ = busca_lay.HitTestTextPosition(pos16 as u32, false, &mut cx, &mut cy, &mut m);
            }
            cursor_busca = cx;
        }

        // Conteúdo.
        let faixa = self.faixa_visivel();
        let mut itens_desenho = Vec::new(); // (k, ret, layout do nome, bitmap, selecionado, extra da lista)
        let colunas = self.colunas_lista();
        let renomeando = self.renomeando.as_ref().map(|(i, _)| *i);
        for k in faixa {
            let i = self.visiveis[k];
            let e = &self.itens[i];
            let r = self.ret_item(k);
            zonas.push((r, Alvo::Item(k)));
            let sel = self.selecao.contains(&i);
            let b = self.bitmaps.get(&e.chave).cloned().flatten();
            match self.modo {
                Modo::Icones => {
                    let nome = lay(&e.nome, &self.rotulo, CEL_L - 12.0, 34.0)?;
                    itens_desenho.push((k, i, r, nome, b, sel, Vec::new()));
                }
                Modo::Colunas => {
                    let nome = lay(&e.nome, &self.texto, COL_W - 60.0, LINHA)?;
                    itens_desenho.push((k, i, r, nome, b, sel, Vec::new()));
                }
                Modo::Lista => {
                    let nome = lay(&e.nome, &self.texto, colunas[0].1 - colunas[0].0 - 34.0, LINHA)?;
                    let data = lay(&arquivos::data_legivel(e.modificado), &self.pequeno, colunas[1].1 - colunas[1].0 - 8.0, LINHA)?;
                    let tam = lay(&if e.pasta { "—".into() } else { arquivos::tamanho_legivel(e.tamanho) }, &self.pequeno, colunas[2].1 - colunas[2].0 - 8.0, LINHA)?;
                    let tipo = lay(&e.tipo, &self.pequeno, colunas[3].1 - colunas[3].0 - 8.0, LINHA)?;
                    itens_desenho.push((k, i, r, nome, b, sel, vec![data, tam, tipo]));
                }
            }
        }
        // Quem recebe arquivos soltos precisa saber onde estão as pastas.
        let destaque = {
            let e = self.escala();
            let mut m = self.mapa.borrow_mut();
            m.pasta = self.pasta.clone();
            m.pastas = itens_desenho
                .iter()
                .filter(|d| self.itens[d.1].pasta)
                .map(|d| {
                    let r = d.2;
                    let r = retangulo(r.left, r.top.max(area.top), r.right, r.bottom.min(area.bottom));
                    (
                        RECT { left: (r.left * e) as i32, top: (r.top * e) as i32, right: (r.right * e) as i32, bottom: (r.bottom * e) as i32 },
                        self.itens[d.1].caminho.clone(),
                    )
                })
                .collect();
            m.destaque.clone()
        };
        let destaque_i = destaque.and_then(|p| self.itens.iter().position(|e| e.caminho == p));

        // Modo Colunas: pastas acima da atual e a prévia do arquivo escolhido.
        let itens_pasta: Vec<bool> = self.itens.iter().map(|e| e.pasta).collect();
        let x_atual = self.x_coluna_atual();
        let mut cols_desenho = Vec::new();
        let mut previa_coluna = None;
        if self.modo == Modo::Colunas {
            let nvis = self.ancestrais_visiveis();
            let primeira = self.ancestrais.len() - nvis;
            let cabem = (((area.bottom - area.top - 12.0) / LINHA).floor() as usize).max(1);
            for (c, j) in (primeira..self.ancestrais.len()).enumerate() {
                let (_, itens, no_caminho) = &self.ancestrais[j];
                let x0 = area.left + c as f32 * COL_W;
                let marcado = itens.iter().position(|e| Some(&e.caminho) == no_caminho.as_ref());
                let inicio = marcado.map_or(0, |m| m.saturating_sub(cabem / 2)).min(itens.len().saturating_sub(cabem));
                let mut linhas_col = Vec::new();
                for (n, idx) in (inicio..itens.len().min(inicio + cabem + 1)).enumerate() {
                    let e = &itens[idx];
                    let y = area.top + 6.0 + n as f32 * LINHA;
                    let r = retangulo(x0 + 5.0, y, x0 + COL_W - 5.0, y + LINHA);
                    zonas.push((r, Alvo::ColunaItem(j, idx)));
                    let l = lay(&e.nome, &self.texto, COL_W - 60.0, LINHA)?;
                    linhas_col.push((r, l, self.bitmaps.get(&e.chave).cloned().flatten(), Some(idx) == marcado, e.pasta));
                }
                cols_desenho.push((x0, linhas_col));
            }
            let sel: Vec<usize> = self.selecao.iter().copied().collect();
            if let [i] = sel[..] {
                let e = &self.itens[i];
                let livre = area.right - (x_atual + COL_W);
                if !e.pasta && livre > 180.0 {
                    let info = format!(
                        "{}\n{}\n{}\n{}",
                        e.nome,
                        e.tipo,
                        arquivos::tamanho_legivel(e.tamanho),
                        arquivos::data_legivel(e.modificado)
                    );
                    let mut l = lay(&info, &self.rotulo, livre - 40.0, 120.0)?;
                    let _ = &mut l;
                    previa_coluna = Some((l, self.bitmaps.get(&e.chave).cloned().flatten()));
                }
            }
        }

        // Abas (só com duas ou mais).
        let mut abas_desenho = Vec::new(); // (ret, texto, ativa, k)
        let mut nova_aba_r = None;
        if self.abas.len() > 1 {
            let n = self.abas.len();
            let (x0, x1) = (LATERAL, larg - 34.0);
            let w = (x1 - x0) / n as f32;
            for k in 0..n {
                let r = retangulo(x0 + k as f32 * w, TOPO, x0 + (k + 1) as f32 * w, TOPO + ABAS);
                let nome = if k == self.aba { self.nome_da_pasta() } else { self.abas[k].nome() };
                zonas.push((r, Alvo::Aba(k)));
                zonas.push((retangulo(r.left + 6.0, TOPO + 7.0, r.left + 22.0, TOPO + 23.0), Alvo::FecharAba(k)));
                abas_desenho.push((r, lay(&nome, &self.centro, (w - 52.0).max(10.0), ABAS)?, k == self.aba, k));
            }
            let r = retangulo(larg - 31.0, TOPO + 4.0, larg - 7.0, TOPO + ABAS - 4.0);
            zonas.push((r, Alvo::NovaAba));
            nova_aba_r = Some(r);
        }

        let mut cab = Vec::new();
        if self.modo == Modo::Lista {
            for (c, (x0, x1, nome, col)) in colunas.iter().enumerate() {
                let seta = if self.ordem.0 == *col { if self.ordem.1 { "  ▲" } else { "  ▼" } } else { "" };
                cab.push((lay(&format!("{nome}{seta}"), &self.secao, x1 - x0 - 8.0, CABECALHO)?, *x0));
                zonas.push((retangulo(*x0, area.top, *x1, area.top + CABECALHO), Alvo::Cabecalho(c)));
            }
        }
        let vazio = if let Some(e) = &self.erro {
            Some(lay(e, &self.texto, 400.0, 30.0)?)
        } else if self.visiveis.is_empty() {
            let t = if self.sub.is_some() {
                crate::idioma::t("Buscando…", "Searching…")
            } else if self.busca.texto.is_empty() {
                crate::idioma::t("Pasta vazia", "Empty folder")
            } else {
                crate::idioma::t("Nada encontrado", "Nothing found")
            };
            Some(lay(t, &self.texto, 400.0, 30.0)?)
        } else {
            None
        };
        let rename_lay = match &self.renomeando {
            Some((_, c)) => Some((lay(&c.string(), &self.texto, 400.0, 22.0)?, c.tudo)),
            None => None,
        };

        // Rodapé: caminho clicável e contagem.
        let mut caminho = Vec::new();
        let mut cx = LATERAL + 14.0;
        if self.sub.is_none() {
            let segs = self.segmentos_caminho();
            let n = segs.len();
            let inicio = n.saturating_sub(5);
            for (i, (nome, _)) in segs.iter().enumerate().skip(inicio) {
                let l = lay(nome, &self.pequeno, 160.0, RODAPE)?;
                let mut m = DWRITE_TEXT_METRICS::default();
                unsafe {
                    let _ = l.GetMetrics(&mut m);
                }
                zonas.push((retangulo(cx - 4.0, alt - RODAPE, cx + m.width + 4.0, alt), Alvo::Caminho(i)));
                caminho.push((l, cx, i + 1 < n));
                cx += m.width + 22.0;
            }
        }
        let livre = arquivos::espaco_livre(&self.pasta).map(|(l, _)| format!(", {} {}", arquivos::tamanho_legivel(l), crate::idioma::t("disponíveis", "available"))).unwrap_or_default();
        let contagem = if self.selecao.is_empty() {
            format!("{} {}{}", self.visiveis.len(), crate::idioma::t("itens", "items"), livre)
        } else {
            format!("{} {} {} {}{}", self.selecao.len(), crate::idioma::t("de", "of"), self.visiveis.len(), crate::idioma::t("selecionados", "selected"), livre)
        };
        let contagem = lay(&contagem, &self.pequeno, 320.0, RODAPE)?;

        // Pré-visualização.
        let previa = match &self.previa {
            Some(p) => {
                let e = &self.itens[p.item];
                let info = format!(
                    "{}\n{}{}  ·  {}",
                    e.nome,
                    e.tipo,
                    if e.pasta { String::new() } else { format!("  ·  {}", arquivos::tamanho_legivel(e.tamanho)) },
                    arquivos::data_legivel(e.modificado)
                );
                let linhas = match &p.texto {
                    Some(v) => Some(lay(&v.join("\n"), &self.mono, larg * 0.7, alt * 0.7)?),
                    None => None,
                };
                Some((lay(&info, &self.texto, larg * 0.7, 44.0)?, linhas, self.bitmaps.get(&p.chave).cloned().flatten(), self.bitmaps.get(&e.chave).cloned().flatten()))
            }
            None => None,
        };

        let p = &self.pincel;
        let hover = self.hover;
        let modo = self.modo;
        let busca_ativa = self.busca_ativa;
        let busca_vazia = self.busca.texto.is_empty();
        let pode_voltar = self.pos > 0;
        let pode_avancar = self.pos + 1 < self.historico.len();
        let rolagem = self.rolagem.x;
        let alt_conteudo = self.altura_conteudo();
        self.zonas = zonas;
        self.sup.desenhar(gpu, |dc| unsafe {
            // Conteúdo opaco à direita (a lateral fica no vidro).
            p.SetColor(&crate::tema::fundo(0.115, 0.115, 0.13, 0.97));
            dc.FillRectangle(&retangulo(LATERAL, 0.0, larg, alt), p);
            p.SetColor(&cor(0.0, 0.0, 0.0, 0.35));
            dc.FillRectangle(&retangulo(LATERAL, 0.0, LATERAL + 1.0, alt), p);
            p.SetColor(&crate::tema::tinta(0.07));
            dc.FillRectangle(&retangulo(LATERAL, TOPO - 1.0, larg, TOPO), p);
            dc.FillRectangle(&retangulo(LATERAL, alt - RODAPE, larg, alt - RODAPE + 1.0), p);

            for (x, a, rgb) in botoes {
                let op = if ativa { 1.0 } else { 0.35 };
                p.SetColor(&cor(rgb.0, rgb.1, rgb.2, op));
                dc.FillEllipse(&D2D1_ELLIPSE { point: pt(x, 20.0), radiusX: 6.0, radiusY: 6.0 }, p);
                if hover.is_some_and(|h| matches!(h, Alvo::Fechar | Alvo::Minimizar | Alvo::Maximizar)) {
                    p.SetColor(&cor(0.0, 0.0, 0.0, 0.55));
                    match a {
                        Alvo::Fechar => {
                            dc.DrawLine(pt(x - 2.5, 17.5), pt(x + 2.5, 22.5), p, 1.2, None);
                            dc.DrawLine(pt(x - 2.5, 22.5), pt(x + 2.5, 17.5), p, 1.2, None);
                        }
                        Alvo::Minimizar => dc.DrawLine(pt(x - 3.0, 20.0), pt(x + 3.0, 20.0), p, 1.2, None),
                        _ => {
                            dc.DrawLine(pt(x - 3.0, 20.0), pt(x + 3.0, 20.0), p, 1.2, None);
                            dc.DrawLine(pt(x, 17.0), pt(x, 23.0), p, 1.2, None);
                        }
                    }
                }
            }

            // Lateral.
            for (l, y, lugar) in &lateral {
                match lugar {
                    None => {
                        p.SetColor(&crate::tema::tinta(0.45));
                        dc.DrawTextLayout(pt(18.0, *y), l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                    }
                    Some(k) => {
                        let r = retangulo(10.0, *y, LATERAL - 10.0, y + 26.0);
                        if lugar_atual == Some(*k) {
                            p.SetColor(&crate::tema::tinta(0.16));
                            dc.FillRoundedRectangle(&arredondado(r, 6.0), p);
                        } else if hover == Some(Alvo::Lugar(*k)) {
                            p.SetColor(&crate::tema::tinta(0.07));
                            dc.FillRoundedRectangle(&arredondado(r, 6.0), p);
                        }
                        let (ix, iy) = (28.0, y + 13.0);
                        p.SetColor(&crate::tema::destaque_vivo(0.95));
                        if eh_disco(*k) {
                            dc.DrawRoundedRectangle(&arredondado(retangulo(ix - 7.0, iy - 4.5, ix + 7.0, iy + 4.5), 2.0), p, 1.3, None);
                            dc.FillEllipse(&D2D1_ELLIPSE { point: pt(ix + 4.0, iy + 1.5), radiusX: 1.0, radiusY: 1.0 }, p);
                        } else {
                            icone_pasta(dc, p, ix, iy, 0.8);
                        }
                        p.SetColor(&crate::tema::tinta(0.92));
                        dc.DrawTextLayout(pt(42.0, *y), l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                    }
                }
            }

            // Barra de ferramentas.
            for (r, ativo, frente) in [(retangulo(tx, 12.0, tx + 28.0, 40.0), pode_voltar, false), (retangulo(tx + 30.0, 12.0, tx + 58.0, 40.0), pode_avancar, true)] {
                if ativo && hover.is_some_and(|h| (h == Alvo::Voltar && !frente) || (h == Alvo::Avancar && frente)) {
                    p.SetColor(&crate::tema::tinta(0.1));
                    dc.FillRoundedRectangle(&arredondado(r, 6.0), p);
                }
                let (cx, cy) = ((r.left + r.right) / 2.0, 26.0);
                p.SetColor(&crate::tema::tinta(if ativo { 0.85 } else { 0.25 }));
                let d = if frente { 1.0 } else { -1.0 };
                dc.DrawLine(pt(cx - 3.0 * d, cy - 6.0), pt(cx + 3.0 * d, cy), p, 1.8, None);
                dc.DrawLine(pt(cx + 3.0 * d, cy), pt(cx - 3.0 * d, cy + 6.0), p, 1.8, None);
            }
            p.SetColor(&crate::tema::tinta(0.95));
            dc.DrawTextLayout(pt(tx + 70.0, 0.0), &titulo, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            for (r, m) in [(modo_i, Modo::Icones), (modo_l, Modo::Lista), (modo_c, Modo::Colunas)] {
                if modo == m {
                    p.SetColor(&crate::tema::tinta(0.14));
                    dc.FillRoundedRectangle(&arredondado(r, 6.0), p);
                }
                p.SetColor(&crate::tema::tinta(0.85));
                let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                if m == Modo::Icones {
                    for (dx, dy) in [(-3.5f32, -3.5f32), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
                        dc.FillRoundedRectangle(&arredondado(retangulo(cx + dx - 2.5, cy + dy - 2.5, cx + dx + 2.5, cy + dy + 2.5), 1.0), p);
                    }
                } else if m == Modo::Colunas {
                    dc.DrawRoundedRectangle(&arredondado(retangulo(cx - 7.5, cy - 6.0, cx + 7.5, cy + 6.0), 2.0), p, 1.3, None);
                    for dx in [-2.5f32, 2.5] {
                        dc.FillRectangle(&retangulo(cx + dx - 0.6, cy - 6.0, cx + dx + 0.6, cy + 6.0), p);
                    }
                } else {
                    for dy in [-4.5f32, 0.0, 4.5] {
                        dc.FillRectangle(&retangulo(cx - 7.0, cy + dy - 0.8, cx + 7.0, cy + dy + 0.8), p);
                    }
                }
            }
            p.SetColor(&crate::tema::tinta(if busca_ativa { 0.14 } else { 0.08 }));
            dc.FillRoundedRectangle(&arredondado(busca_r, 7.0), p);
            if busca_ativa {
                p.SetColor(&crate::tema::destaque_a(0.9));
                dc.DrawRoundedRectangle(&arredondado(busca_r, 7.0), p, 1.5, None);
            }
            icones::lupa(dc, p, busca_r.left + 15.0, 26.0, 0.75, crate::tema::tinta(0.55));
            p.SetColor(&crate::tema::tinta(if busca_vazia { 0.45 } else { 0.95 }));
            dc.DrawTextLayout(pt(busca_r.left + 28.0, 12.0), &busca_lay, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            if busca_ativa {
                p.SetColor(&crate::tema::destaque_vivo(1.0));
                let xc = busca_r.left + 28.0 + cursor_busca;
                dc.FillRectangle(&retangulo(xc, 18.0, xc + 1.5, 34.0), p);
            }

            // Abas.
            if !abas_desenho.is_empty() {
                p.SetColor(&crate::tema::fundo(0.075, 0.075, 0.085, 1.0));
                dc.FillRectangle(&retangulo(LATERAL + 1.0, TOPO, larg, TOPO + ABAS), p);
                for (r, nome, ativa_aba, k) in &abas_desenho {
                    let sobre = hover == Some(Alvo::Aba(*k)) || hover == Some(Alvo::FecharAba(*k));
                    if *ativa_aba {
                        p.SetColor(&crate::tema::fundo(0.115, 0.115, 0.13, 1.0));
                        dc.FillRectangle(r, p);
                    } else if sobre {
                        p.SetColor(&crate::tema::tinta(0.04));
                        dc.FillRectangle(r, p);
                    }
                    if *k > 0 {
                        p.SetColor(&crate::tema::tinta(0.08));
                        dc.FillRectangle(&retangulo(r.left, r.top + 6.0, r.left + 1.0, r.bottom - 6.0), p);
                    }
                    p.SetColor(&crate::tema::tinta(if *ativa_aba { 0.92 } else { 0.55 }));
                    dc.DrawTextLayout(pt(r.left + 26.0, r.top), nome, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                    if sobre {
                        let (cx, cy) = (r.left + 14.0, r.top + ABAS / 2.0);
                        if hover == Some(Alvo::FecharAba(*k)) {
                            p.SetColor(&crate::tema::tinta(0.12));
                            dc.FillRoundedRectangle(&arredondado(retangulo(cx - 8.0, cy - 8.0, cx + 8.0, cy + 8.0), 4.0), p);
                        }
                        p.SetColor(&crate::tema::tinta(0.7));
                        dc.DrawLine(pt(cx - 3.5, cy - 3.5), pt(cx + 3.5, cy + 3.5), p, 1.3, None);
                        dc.DrawLine(pt(cx - 3.5, cy + 3.5), pt(cx + 3.5, cy - 3.5), p, 1.3, None);
                    }
                }
                if let Some(r) = nova_aba_r {
                    if hover == Some(Alvo::NovaAba) {
                        p.SetColor(&crate::tema::tinta(0.1));
                        dc.FillRoundedRectangle(&arredondado(r, 5.0), p);
                    }
                    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                    p.SetColor(&crate::tema::tinta(0.7));
                    dc.DrawLine(pt(cx - 5.0, cy), pt(cx + 5.0, cy), p, 1.4, None);
                    dc.DrawLine(pt(cx, cy - 5.0), pt(cx, cy + 5.0), p, 1.4, None);
                }
                p.SetColor(&crate::tema::tinta(0.07));
                dc.FillRectangle(&retangulo(LATERAL, TOPO + ABAS - 1.0, larg, TOPO + ABAS), p);
            }

            // Conteúdo.
            dc.PushAxisAlignedClip(&area, D2D1_ANTIALIAS_MODE_ALIASED);
            // Colunas: as pastas acima da atual, cada uma com o item do
            // caminho destacado, e um traço separando as colunas.
            for (x0, linhas_col) in &cols_desenho {
                for (r, l, b, no_caminho, pasta) in linhas_col {
                    if *no_caminho {
                        p.SetColor(&crate::tema::tinta(0.13));
                        dc.FillRoundedRectangle(&arredondado(*r, 5.0), p);
                    }
                    let cy = (r.top + r.bottom) / 2.0;
                    desenhar_icone(dc, p, b.as_ref(), retangulo(r.left + 6.0, cy - ICONE_P / 2.0, r.left + 6.0 + ICONE_P, cy + ICONE_P / 2.0));
                    p.SetColor(&crate::tema::tinta(0.9));
                    dc.DrawTextLayout(pt(r.left + 32.0, r.top), l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                    if *pasta {
                        p.SetColor(&crate::tema::tinta(0.45));
                        seta_direita(dc, p, r.right - 12.0, cy);
                    }
                }
                p.SetColor(&crate::tema::tinta(0.08));
                dc.FillRectangle(&retangulo(x0 + COL_W - 0.5, area.top, x0 + COL_W + 0.5, area.bottom), p);
            }
            if modo == Modo::Colunas {
                let x1 = x_atual + COL_W;
                p.SetColor(&crate::tema::tinta(0.08));
                dc.FillRectangle(&retangulo(x1 - 0.5, area.top, x1 + 0.5, area.bottom), p);
                if let Some((l, b)) = &previa_coluna {
                    let cx = (x1 + area.right) / 2.0;
                    let ic = retangulo(cx - 48.0, area.top + 40.0, cx + 48.0, area.top + 136.0);
                    desenhar_icone(dc, p, b.as_ref(), ic);
                    p.SetColor(&crate::tema::tinta(0.85));
                    dc.DrawTextLayout(pt(x1 + 20.0, area.top + 150.0), l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                }
            }
            if modo == Modo::Lista {
                // Listras alternadas e cabeçalho.
                for (k, _, r, ..) in &itens_desenho {
                    if k % 2 == 1 {
                        p.SetColor(&crate::tema::tinta(0.025));
                        dc.FillRectangle(r, p);
                    }
                }
            }
            for (_, i, r, nome, b, sel, extras) in &itens_desenho {
                match modo {
                    Modo::Icones => {
                        let ic = retangulo((r.left + r.right) / 2.0 - ICONE_G / 2.0, r.top + 8.0, (r.left + r.right) / 2.0 + ICONE_G / 2.0, r.top + 8.0 + ICONE_G);
                        if *sel {
                            p.SetColor(&crate::tema::tinta(0.12));
                            dc.FillRoundedRectangle(&arredondado(retangulo(ic.left - 6.0, ic.top - 5.0, ic.right + 6.0, ic.bottom + 5.0), 8.0), p);
                        }
                        desenhar_icone(dc, p, b.as_ref(), ic);
                        let mut m = DWRITE_TEXT_METRICS::default();
                        let _ = nome.GetMetrics(&mut m);
                        let ly = ic.bottom + 9.0;
                        if Some(*i) == renomeando {
                            continue;
                        }
                        if *sel {
                            let cx = (r.left + r.right) / 2.0;
                            p.SetColor(&crate::tema::destaque_a(if ativa { 1.0 } else { 0.45 }));
                            dc.FillRoundedRectangle(&arredondado(retangulo(cx - m.width / 2.0 - 5.0, ly - 1.0, cx + m.width / 2.0 + 5.0, ly + m.height + 1.0), 4.0), p);
                        }
                        p.SetColor(&crate::tema::tinta_sobre(*sel && ativa, 0.93));
                        dc.DrawTextLayout(pt(r.left + 6.0, ly), nome, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                    }
                    Modo::Colunas => {
                        if *sel {
                            p.SetColor(&crate::tema::destaque_a(if ativa { 0.95 } else { 0.4 }));
                            dc.FillRoundedRectangle(&arredondado(*r, 5.0), p);
                        }
                        let cy = (r.top + r.bottom) / 2.0;
                        desenhar_icone(dc, p, b.as_ref(), retangulo(r.left + 6.0, cy - ICONE_P / 2.0, r.left + 6.0 + ICONE_P, cy + ICONE_P / 2.0));
                        if Some(*i) != renomeando {
                            p.SetColor(&crate::tema::tinta_sobre(*sel && ativa, 0.93));
                            dc.DrawTextLayout(pt(r.left + 32.0, r.top), nome, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                        }
                        if itens_pasta.get(*i).copied().unwrap_or(false) {
                            p.SetColor(&crate::tema::tinta_sobre(*sel && ativa, 0.5));
                            seta_direita(dc, p, r.right - 12.0, cy);
                        }
                    }
                    Modo::Lista => {
                        if *sel {
                            p.SetColor(&crate::tema::destaque_a(if ativa { 0.95 } else { 0.4 }));
                            dc.FillRoundedRectangle(&arredondado(*r, 5.0), p);
                        }
                        let cy = (r.top + r.bottom) / 2.0;
                        desenhar_icone(dc, p, b.as_ref(), retangulo(r.left + 6.0, cy - ICONE_P / 2.0, r.left + 6.0 + ICONE_P, cy + ICONE_P / 2.0));
                        if Some(*i) != renomeando {
                            p.SetColor(&crate::tema::tinta_sobre(*sel && ativa, 0.93));
                            dc.DrawTextLayout(pt(r.left + 32.0, r.top), nome, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                        }
                        p.SetColor(&crate::tema::tinta_sobre(*sel && ativa, if *sel { 0.85 } else { 0.55 }));
                        for (c, l) in extras.iter().enumerate() {
                            dc.DrawTextLayout(pt(colunas[c + 1].0 + 4.0, r.top), l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                        }
                    }
                }
            }
            // Pasta que vai receber o que está sendo arrastado.
            if let Some(d) = itens_desenho.iter().find(|d| Some(d.1) == destaque_i) {
                p.SetColor(&crate::tema::destaque_a(0.25));
                dc.FillRoundedRectangle(&arredondado(d.2, 8.0), p);
                p.SetColor(&crate::tema::destaque_vivo(1.0));
                dc.DrawRoundedRectangle(&arredondado(d.2, 8.0), p, 2.0, None);
            }
            // Campo de renomear por cima do nome.
            if let (Some(i), Some((l, tudo))) = (renomeando, &rename_lay) {
                if let Some((_, _, r, ..)) = itens_desenho.iter().find(|d| d.1 == i) {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    let _ = l.GetMetrics(&mut m);
                    let (x0, y0) = match modo {
                        Modo::Icones => ((r.left + r.right) / 2.0 - (m.width / 2.0).min(r.right - r.left), r.top + 8.0 + ICONE_G + 8.0),
                        Modo::Lista | Modo::Colunas => (r.left + 30.0, r.top + 2.0),
                    };
                    let caixa = retangulo(x0 - 4.0, y0 - 1.0, x0 + m.width + 6.0, y0 + 21.0);
                    p.SetColor(&crate::tema::fundo(0.08, 0.08, 0.1, 1.0));
                    dc.FillRoundedRectangle(&arredondado(caixa, 4.0), p);
                    p.SetColor(&crate::tema::destaque_a(1.0));
                    if *tudo {
                        dc.FillRectangle(&retangulo(x0, y0 + 1.0, x0 + m.width, y0 + 19.0), p);
                    }
                    dc.DrawRoundedRectangle(&arredondado(caixa, 4.0), p, 1.5, None);
                    p.SetColor(&crate::tema::tinta_sobre(*tudo, 1.0));
                    dc.DrawTextLayout(pt(x0, y0 - 1.0), l, p, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    if !*tudo {
                        dc.FillRectangle(&retangulo(x0 + m.widthIncludingTrailingWhitespace, y0 + 2.0, x0 + m.widthIncludingTrailingWhitespace + 1.5, y0 + 18.0), p);
                    }
                }
            }
            if modo == Modo::Lista {
                p.SetColor(&crate::tema::fundo(0.115, 0.115, 0.13, 1.0));
                dc.FillRectangle(&retangulo(area.left, area.top, area.right, area.top + CABECALHO), p);
                p.SetColor(&crate::tema::tinta(0.07));
                dc.FillRectangle(&retangulo(area.left, area.top + CABECALHO - 1.0, area.right, area.top + CABECALHO), p);
                p.SetColor(&crate::tema::tinta(0.6));
                for (l, x0) in &cab {
                    dc.DrawTextLayout(pt(x0 + 4.0, area.top), l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                }
            }
            if let Some(l) = &vazio {
                p.SetColor(&crate::tema::tinta(0.4));
                dc.DrawTextLayout(pt((area.left + area.right) / 2.0 - 200.0, (area.top + area.bottom) / 2.0 - 15.0), l, p, D2D1_DRAW_TEXT_OPTIONS_NONE);
            }
            // Barra de rolagem fina.
            let vis = area.bottom - area.top;
            if alt_conteudo > vis + 1.0 {
                let h = (vis * vis / alt_conteudo).max(30.0);
                let y = area.top + (vis - h) * (rolagem / (alt_conteudo - vis)).clamp(0.0, 1.0);
                p.SetColor(&crate::tema::tinta(0.22));
                dc.FillRoundedRectangle(&arredondado(retangulo(area.right - 7.0, y + 2.0, area.right - 3.0, y + h - 2.0), 2.0), p);
            }
            dc.PopAxisAlignedClip();

            // Rodapé.
            for (l, x, seta) in &caminho {
                p.SetColor(&crate::tema::tinta(0.65));
                dc.DrawTextLayout(pt(*x, alt - RODAPE), l, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                let _ = seta;
            }
            for (k, (l, x, seta)) in caminho.iter().enumerate() {
                if *seta {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    let _ = l.GetMetrics(&mut m);
                    let sx = x + m.width + 11.0;
                    p.SetColor(&crate::tema::tinta(0.35));
                    icones::seta(dc, p, sx, alt - RODAPE / 2.0, false);
                }
                let _ = k;
            }
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = contagem.GetMetrics(&mut m);
            p.SetColor(&crate::tema::tinta(0.45));
            dc.DrawTextLayout(pt(larg - 14.0 - m.width, alt - RODAPE), &contagem, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);

            // Pré-visualização (Espaço).
            if let Some((info, linhas, grande, pequeno)) = &previa {
                p.SetColor(&cor(0.0, 0.0, 0.0, 0.55));
                dc.FillRectangle(&retangulo(0.0, 0.0, larg, alt), p);
                let caixa = retangulo(larg * 0.12, alt * 0.08, larg * 0.88, alt * 0.92);
                p.SetColor(&crate::tema::fundo(0.16, 0.16, 0.18, 0.98));
                dc.FillRoundedRectangle(&arredondado(caixa, 12.0), p);
                p.SetColor(&crate::tema::tinta(0.12));
                dc.DrawRoundedRectangle(&arredondado(caixa, 12.0), p, 1.0, None);
                let corpo = retangulo(caixa.left + 20.0, caixa.top + 16.0, caixa.right - 20.0, caixa.bottom - 64.0);
                if let Some(l) = linhas {
                    dc.PushAxisAlignedClip(&corpo, D2D1_ANTIALIAS_MODE_ALIASED);
                    p.SetColor(&cor(0.9, 0.9, 0.92, 1.0));
                    dc.DrawTextLayout(pt(corpo.left, corpo.top), l, p, D2D1_DRAW_TEXT_OPTIONS_NONE);
                    dc.PopAxisAlignedClip();
                } else if let Some(b) = grande.as_ref().or(pequeno.as_ref()) {
                    let s = b.GetSize();
                    let k = ((corpo.right - corpo.left) / s.width).min((corpo.bottom - corpo.top) / s.height).min(if grande.is_some() { 1.0 } else { 2.0 });
                    let (w, h) = (s.width * k, s.height * k);
                    let cx = (corpo.left + corpo.right) / 2.0;
                    let cy = (corpo.top + corpo.bottom) / 2.0;
                    dc.DrawBitmap(b, Some(&retangulo(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0)), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
                }
                p.SetColor(&crate::tema::tinta(0.9));
                dc.DrawTextLayout(pt(caixa.left + 20.0, caixa.bottom - 56.0), info, p, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            }
        })?;
        self.sujo = false;
        Ok(())
    }
}

/// Ícone do arquivo (ou um quadradinho enquanto ele carrega).
unsafe fn desenhar_icone(dc: &ID2D1DeviceContext, p: &ID2D1SolidColorBrush, b: Option<&ID2D1Bitmap1>, r: D2D_RECT_F) {
    match b {
        Some(b) => {
            // Miniaturas não são quadradas: cabem no quadrado sem deformar.
            let s = b.GetSize();
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let k = (w / s.width).min(h / s.height);
            let (bw, bh) = (s.width * k, s.height * k);
            let cx = (r.left + r.right) / 2.0;
            let base = r.bottom;
            dc.DrawBitmap(b, Some(&retangulo(cx - bw / 2.0, base - bh - (h - bh) / 2.0, cx + bw / 2.0, base - (h - bh) / 2.0)), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
        }
        None => {
            p.SetColor(&crate::tema::tinta(0.06));
            dc.FillRoundedRectangle(&arredondado(r, (r.right - r.left) * 0.15), p);
        }
    }
}

/// Pastinha desenhada à mão (barra lateral).
/// Setinha ">" (pasta no modo Colunas).
unsafe fn seta_direita(dc: &ID2D1DeviceContext, p: &ID2D1SolidColorBrush, x: f32, y: f32) {
    dc.DrawLine(pt(x - 2.5, y - 4.5), pt(x + 1.5, y), p, 1.4, None);
    dc.DrawLine(pt(x + 1.5, y), pt(x - 2.5, y + 4.5), p, 1.4, None);
}

unsafe fn icone_pasta(dc: &ID2D1DeviceContext, p: &ID2D1SolidColorBrush, cx: f32, cy: f32, t: f32) {
    dc.DrawRoundedRectangle(&arredondado(retangulo(cx - 8.0 * t, cy - 4.0 * t, cx + 8.0 * t, cy + 6.0 * t), 2.0 * t), p, 1.3, None);
    dc.DrawLine(pt(cx - 8.0 * t, cy - 4.0 * t), pt(cx - 8.0 * t, cy - 6.5 * t), p, 1.3, None);
    dc.DrawLine(pt(cx - 8.0 * t, cy - 6.5 * t), pt(cx - 2.0 * t, cy - 6.5 * t), p, 1.3, None);
    dc.DrawLine(pt(cx - 2.0 * t, cy - 6.5 * t), pt(cx, cy - 4.0 * t), p, 1.3, None);
}

/// Pasta inicial: a pasta pessoal do usuário.
pub fn pasta_inicial() -> PathBuf {
    arquivos::favoritos()
        .into_iter()
        .find(|(n, _)| n == crate::idioma::t("Pasta pessoal", "Home"))
        .map(|(_, p)| p)
        .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("C:\\"))
}

