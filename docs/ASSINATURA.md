# Assinatura digital do cais.exe

## Por que

Um `.exe` novo, sem assinatura, que mexe em janelas de outros programas e simula teclas (o Cais faz isso para o gênio, o alternador e os cantos ativos) chama a atenção de antivírus por heurística. O Kaspersky, por exemplo, já marcou uma versão de desenvolvimento como `VHO:Trojan.Win32.Shellcode.gen`, um falso positivo. O SmartScreen do Windows também avisa "aplicativo não reconhecido".

A assinatura digital diz quem publicou o arquivo e garante que ele não foi alterado. Com o tempo, a reputação do certificado faz os avisos sumirem.

## Como (gratuito para open source)

1. O repositório precisa estar **público**.
2. Pedir a entrada no **SignPath Foundation** (https://signpath.org), que assina projetos open source sem custo. Eles pedem:
   - licença aprovada pela OSI (o Cais é MIT);
   - builds feitas pela CI do GitHub, não na máquina de alguém (o `release.yml` já faz isso);
   - uma política de assinatura que só assina versões marcadas com tag.
3. Depois da aprovação, criar no GitHub:
   - o segredo `SIGNPATH_API_TOKEN`;
   - a variável `SIGNPATH_ORGANIZATION_ID`.
4. Descomentar o passo "Assinar" em `.github/workflows/release.yml`.

## Enquanto não tem assinatura

- Quem baixar o `.zip` pode conferir o SHA-256 publicado junto na Release.
- Se o antivírus barrar, a saída segura é adicionar a pasta do Cais como exceção (e, se quiser, mandar o arquivo para o fabricante analisar o falso positivo).
- Nunca aceite "desinfectar e reiniciar" num arquivo que você mesmo compilou sem antes entender o alerta.
