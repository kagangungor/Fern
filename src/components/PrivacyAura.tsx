import { useLayoutEffect, useRef, useState } from "react";

interface Props {
  flare: number;
  radius?: number;
  type: "mic" | "cam" | "both";
}

const INSET = 0.5;

function silhouette(
  w: number,
  h: number,
  f: number,
  r: number,
  i: number
): string {
  const clamp = (v: number, max: number) => Math.max(0, Math.min(v, max));
  const rr = clamp(r, Math.min(w / 2, h));
  const a = Math.max(0, rr - i);

  if (f > 0) {
    return `M ${-f} 0 A ${f} ${f} 0 0 1 ${i} ${f} V ${h - rr} A ${a} ${a} 0 0 0 ${rr} ${h - i} H ${w - rr} A ${a} ${a} 0 0 0 ${w - i} ${h - rr} V ${f} A ${f} ${f} 0 0 1 ${w + f} 0`;
  }

  const ta = Math.max(0, 18 - i);
  return `M ${i} ${h - rr} V 18 A ${ta} ${ta} 0 0 1 18 ${i} H ${w - 18} A ${ta} ${ta} 0 0 1 ${w - i} 18 V ${h - rr} A ${a} ${a} 0 0 1 ${w - rr} ${h - i} H ${rr} A ${a} ${a} 0 0 1 ${i} ${h - rr} Z`;
}

export function PrivacyAura({ flare, radius = 18, type }: Props) {
  const ref = useRef<SVGSVGElement>(null);
  const [size, setSize] = useState({ w: 0, h: 0 });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const target = el.parentElement || el;
    const measure = () =>
      setSize((prev) =>
        prev.w === target.clientWidth && prev.h === target.clientHeight
          ? prev
          : { w: target.clientWidth, h: target.clientHeight }
      );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(target);
    return () => observer.disconnect();
  }, []);

  const { w, h } = size;
  if (w <= 0 || h <= 0) return null;

  const path = silhouette(w, h, flare, radius, INSET);
  const glowPath = silhouette(w, h, flare, radius, 1.5);

  const filterId = `privacy-glow-filter-${type}`;

  return (
    <svg ref={ref} className={`privacy-aura-svg ${type}`} aria-hidden="true">
      <defs>
        <filter id={filterId} x="-40%" y="-40%" width="180%" height="180%">
          <feGaussianBlur stdDeviation="5" result="blur1" />
          <feGaussianBlur in="SourceGraphic" stdDeviation="2.5" result="blur2" />
          <feMerge>
            <feMergeNode in="blur1" />
            <feMergeNode in="blur2" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </defs>

      {/* Ambient outer halo that traces the flares and notch bottom */}
      <path
        d={glowPath}
        className="aura-path glow"
        filter={`url(#${filterId})`}
      />

      {/* Crisp inner stroke */}
      <path d={path} className="aura-path rim" />
    </svg>
  );
}
