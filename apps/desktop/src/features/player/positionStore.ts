/**
 * Posição de reprodução fora do estado do React.
 *
 * O motor avisa a posição ~30 vezes por segundo. Se isso fosse estado do React, o app inteiro
 * seria redesenhado a cada aviso. Aqui só quem se inscreve (a barra de tempo, a letra) reage.
 *
 * Também resolve o "vai e volta" do seek: depois de pular, avisos que o motor mandou antes de
 * processar o pedido ainda trazem a posição antiga. Enquanto o pedido está "em voo", esses
 * avisos são ignorados e a barra fica no destino.
 */

export interface Store<T> {
  get(): T;
  subscribe(listener: () => void): () => void;
}

/** Quanto o aviso do motor pode diferir do destino para valer como "o seek chegou". */
export const SEEK_TOLERANCE_S = 0.75;
/** Se o motor nunca confirmar (música trocada, erro), solta a trava depois deste tempo. */
export const SEEK_HOLD_MAX_MS = 2000;

export class PositionStore implements Store<number> {
  private value = 0;
  private hold: { target: number; at: number } | null = null;
  private listeners = new Set<() => void>();

  constructor(private now: () => number = () => performance.now()) {}

  get = (): number => this.value;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private emit(): void {
    for (const l of this.listeners) l();
  }

  private assign(v: number): void {
    if (v !== this.value) {
      this.value = v;
      this.emit();
    }
  }

  /** Posição absoluta (música nova, volta ao início): cancela qualquer seek em voo. */
  reset(position: number): void {
    this.hold = null;
    this.assign(position);
  }

  /** O usuário pediu para ir a `target`: a barra mostra o destino já, sem esperar o motor. */
  seekTo(target: number): void {
    this.hold = { target, at: this.now() };
    this.assign(target);
  }

  /** Posição informada pelo motor. */
  fromEngine(position: number): void {
    if (this.hold) {
      const near = Math.abs(position - this.hold.target) <= SEEK_TOLERANCE_S;
      const expired = this.now() - this.hold.at > SEEK_HOLD_MAX_MS;
      if (!near && !expired) return; // aviso velho, de antes do seek
      this.hold = null;
    }
    this.assign(position);
  }
}

export class ValueStore<T> implements Store<T> {
  private listeners = new Set<() => void>();
  constructor(private value: T) {}

  get = (): T => this.value;

  set(v: T): void {
    this.value = v;
    for (const l of this.listeners) l();
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
}
