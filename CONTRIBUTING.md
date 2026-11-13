# Como contribuir

Obrigado por querer ajudar o Cais! *English below.*

## Regras do projeto

1. **Zero rede.** Nenhum código pode abrir conexão, nem para "só checar atualização". O `deny.toml` bloqueia bibliotecas de rede e a CI confere as DLLs importadas pelo executável. PRs que mexem nisso não entram.
2. **Leve de verdade.** Nada de timer rodando à toa: o Cais desenha só quando algo muda. Antes de mandar um PR que mexe em desenho ou eventos, rode:
   ```
   .\target\release\cais.exe --medir 10 --saida medicao.txt
   ```
   e compare a CPU parado (deve continuar perto de 0%).
3. **Nada da Apple.** Sem ícones, fontes, imagens ou nomes de produtos da Apple na interface. Ícones são desenhados no código (`src/icones.rs`).
4. **Português no código, dois idiomas na tela.** Nomes e comentários em português. Todo texto que aparece na tela passa por `idioma::t("português", "English")`.
5. **Não mexe no sistema sem avisar.** O que o Cais muda no Windows (barra de tarefas, animações) volta ao normal quando ele sai.

## Passo a passo

1. Abra uma issue contando o que quer mudar (bug ou ideia).
2. Faça um fork e uma branch a partir de `main`.
3. `cargo build --release` e `cargo test --release` sem erros nem avisos.
4. `.\scripts\verificar-privacidade.ps1` tem que passar.
5. Abra o PR explicando o que mudou e como testou na tela.

---

# Contributing

Thanks for wanting to help!

## Project rules

1. **Zero networking.** No code may open a connection, not even to "just check for updates". `deny.toml` blocks networking crates and CI checks the DLLs imported by the executable.
2. **Actually light.** No idle timers: Cais only draws when something changes. Measure with `cais.exe --medir 10 --saida medicao.txt` before sending changes to drawing or events.
3. **Nothing from Apple.** No Apple icons, fonts, images or product names in the UI. Icons are drawn in code (`src/icones.rs`).
4. **Portuguese in the code, two languages on screen.** Identifiers and comments are in Portuguese. Every on-screen string goes through `idioma::t("português", "English")`.
5. **No silent system changes.** Whatever Cais changes in Windows (taskbar, animations) goes back to normal when it quits.

## Steps

1. Open an issue describing the change (bug or idea).
2. Fork and branch from `main`.
3. `cargo build --release` and `cargo test --release` with no errors or warnings.
4. `.\scripts\verificar-privacidade.ps1` must pass.
5. Open the PR explaining what changed and how you tested it on screen.
