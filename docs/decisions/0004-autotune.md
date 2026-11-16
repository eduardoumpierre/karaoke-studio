# 0004. Autotune

Status: aceita (spike 4)

**Decisão:** YIN (janela de 16 ms, passo de 5 ms) + quantização para a escala com histerese + deslocador no domínio do tempo (linha de atraso com saltos de um período). Custo de 1 a 6% de um núcleo e atraso médio de 9 a 15 ms.

**Arquitetura:** a detecção roda em outra thread; o callback só aplica o deslocador. O monitor do cantor vai sem autotune; a voz corrigida vai só para as caixas. Parâmetros: tom (automático da análise da música ou manual), escala, força (0 a 1) e velocidade (0 a 200 ms).

**A avaliar:** qualidade subjetiva em tempo real (formantes acompanham a altura; correções acima de ~1 semitom podem soar artificiais).
