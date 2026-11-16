export function Slider({
  label,
  value,
  onChange,
  disabled,
  hint,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
  disabled?: boolean;
  hint?: string;
}) {
  return (
    <label className="vol" title={hint}>
      <span>{label}</span>
      <input
        type="range"
        min={0}
        max={100}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        aria-label={label}
      />
      <output>{value}%</output>
    </label>
  );
}

export function Meter({ peak }: { peak: number }) {
  return (
    <div className="meter" role="meter" aria-valuemin={0} aria-valuemax={1} aria-valuenow={peak}>
      <div className="meter-fill" style={{ width: `${Math.min(1, peak) * 100}%` }} />
    </div>
  );
}

/** Controle deslizante genérico (unidade e faixa livres), no mesmo visual do `Slider`. */
export function RangeRow({
  label,
  value,
  min,
  max,
  step = 1,
  format,
  onChange,
  disabled,
  hint,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  format: (v: number) => string;
  onChange: (v: number) => void;
  disabled?: boolean;
  hint?: string;
}) {
  return (
    <label className="vol" title={hint}>
      <span>{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        aria-label={label}
      />
      <output>{format(value)}</output>
    </label>
  );
}
