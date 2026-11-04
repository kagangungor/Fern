import fs from 'node:fs';
import path from 'node:path';

const outDir = path.join('public', 'mascot');
if (!fs.existsSync(outDir)) {
  fs.mkdirSync(outDir, { recursive: true });
}

function createSvg(state, isSit) {
  const C = {
    darkGreen: '#064E3B',
    midGreen: '#059669',
    leafGreen: '#10B981',
    lightGreen: '#34D399',
    brightGreen: '#6EE7B7',
    sproutTip: '#A7F3D0',
    eye: '#022C22',
    eyeShine: '#FFFFFF',
    blush: '#F472B6',
    belly: '#D1FAE5',
    gold: '#FBBF24',
    amber: '#F59E0B',
    red: '#EF4444',
    blue: '#3B82F6',
    purple: '#8B5CF6',
  };

  const P = 20; // 20px per pixel in 360x440 viewBox
  const rect = (x, y, w, h, fill, cls = '') =>
    `<rect x="${x * P}" y="${y * P}" width="${w * P}" height="${h * P}" fill="${fill}"${cls ? ` class="${cls}"` : ''} />`;

  let extraStyles = '';

  if (state === 'working') {
    extraStyles = `
      @keyframes typeL { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-8px); } }
      @keyframes typeR { 0%, 100% { transform: translateY(-8px); } 50% { transform: translateY(0); } }
      @keyframes workGlow { 0%, 100% { opacity: 0.3; } 50% { opacity: 0.9; } }
      .arm-l { animation: typeL 0.25s infinite; }
      .arm-r { animation: typeR 0.25s infinite; }
      .glow { animation: workGlow 1s ease-in-out infinite; }
    `;
  } else if (state === 'needs-input') {
    extraStyles = `
      @keyframes wave { 0%, 100% { transform: rotate(0deg); } 50% { transform: rotate(-15deg); } }
      @keyframes floatQ { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-10px); } }
      .wave-arm { transform-origin: 60px 240px; animation: wave 0.8s ease-in-out infinite; }
      .badge-q { animation: floatQ 1.2s ease-in-out infinite; }
    `;
  } else if (state === 'done') {
    extraStyles = `
      @keyframes cheer { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-16px); } }
      @keyframes spinStar { 0% { opacity: 0.3; transform: scale(0.8); } 50% { opacity: 1; transform: scale(1.1); } 100% { opacity: 0.3; transform: scale(0.8); } }
      .cheer-body { animation: cheer 0.6s ease-in-out infinite; }
      .star { animation: spinStar 1s ease-in-out infinite; }
    `;
  } else if (state === 'error') {
    extraStyles = `
      @keyframes shake { 0%, 100% { transform: translateX(0); } 25% { transform: translateX(-4px); } 75% { transform: translateX(4px); } }
      @keyframes sweat { 0% { opacity: 0; transform: translateY(-5px); } 50% { opacity: 1; } 100% { opacity: 0; transform: translateY(8px); } }
      .shake-body { animation: shake 0.3s ease-in-out infinite; }
      .sweat-drop { animation: sweat 1.2s ease-in-out infinite; }
    `;
  } else if (state === 'sleeping') {
    extraStyles = `
      @keyframes sleepZ { 0% { opacity: 0; transform: translate(0, 0) scale(0.6); } 50% { opacity: 1; } 100% { opacity: 0; transform: translate(20px, -30px) scale(1.2); } }
      .zzz-1 { animation: sleepZ 2s ease-in-out infinite; }
      .zzz-2 { animation: sleepZ 2s ease-in-out 0.7s infinite; }
      .zzz-3 { animation: sleepZ 2s ease-in-out 1.4s infinite; }
    `;
  } else if (state === 'music') {
    extraStyles = `
      @keyframes musicBob { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-10px) rotate(2deg); } }
      @keyframes floatNote { 0% { opacity: 0; transform: translateY(0); } 50% { opacity: 1; } 100% { opacity: 0; transform: translateY(-25px); } }
      .music-body { animation: musicBob 0.6s ease-in-out infinite; }
      .note-1 { animation: floatNote 1.5s ease-in-out infinite; }
      .note-2 { animation: floatNote 1.5s ease-in-out 0.75s infinite; }
    `;
  } else {
    // idle
    extraStyles = `
      @keyframes gentleBob { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-4px); } }
      @keyframes blink { 0%, 90%, 100% { opacity: 1; } 93%, 97% { opacity: 0; } }
      @keyframes blinkClosed { 0%, 90%, 100% { opacity: 0; } 93%, 97% { opacity: 1; } }
      .idle-body { animation: gentleBob 3s ease-in-out infinite; }
      .eye-open { animation: blink 4s step-end infinite; }
      .eye-closed { animation: blinkClosed 4s step-end infinite; }
    `;
  }

  // Sprout atop head (Fern's koru / spiral)
  const sprout = [
    rect(8, 2, 2, 1, C.sproutTip),
    rect(7, 3, 1, 2, C.brightGreen),
    rect(10, 3, 1, 1, C.sproutTip),
    rect(8, 4, 2, 1, C.brightGreen),
    rect(8, 5, 2, 2, C.leafGreen),
    rect(8, 7, 2, 1, C.midGreen),
  ];

  // Head & body
  const body = [
    rect(6, 8, 6, 1, C.midGreen),
    rect(5, 9, 8, 1, C.midGreen),
    rect(4, 10, 10, 1, C.midGreen),
    rect(3, 11, 12, 4, C.midGreen),
    rect(4, 15, 10, 1, C.midGreen),
    rect(5, 16, 8, 1, C.midGreen),

    rect(6, 9, 6, 1, C.lightGreen),
    rect(5, 10, 8, 1, C.lightGreen),
    rect(4, 11, 10, 4, C.lightGreen),
    rect(5, 15, 8, 1, C.lightGreen),

    rect(6, 9, 3, 1, C.brightGreen),
    rect(5, 10, 2, 1, C.brightGreen),

    rect(4, 13, 2, 1, C.blush),
    rect(12, 13, 2, 1, C.blush),

    rect(7, 13, 4, 3, C.belly),
  ];

  const feet = isSit ? [
    rect(3, 16, 3, 1, C.midGreen),
    rect(12, 16, 3, 1, C.midGreen),
    rect(4, 16, 2, 1, C.lightGreen),
    rect(12, 16, 2, 1, C.lightGreen),
  ] : [
    rect(6, 17, 2, 1, C.midGreen),
    rect(10, 17, 2, 1, C.midGreen),
  ];

  let face = [];
  let arms = [];
  let props = [];

  if (state === 'working') {
    face = [
      rect(6, 11, 2, 2, C.eye),
      rect(6, 11, 1, 1, C.eyeShine),
      rect(10, 11, 2, 2, C.eye),
      rect(10, 11, 1, 1, C.eyeShine),
      rect(8, 13, 2, 1, C.darkGreen),
    ];
    arms = [
      rect(3, 13, 2, 2, C.leafGreen, 'arm-l'),
      rect(13, 13, 2, 2, C.leafGreen, 'arm-r'),
    ];
    props = [
      rect(5, 15, 8, 1, '#475569'),
      rect(6, 15, 6, 1, '#94A3B8'),
      rect(6, 14, 1, 1, C.brightGreen, 'glow'),
      rect(11, 14, 1, 1, C.brightGreen, 'glow'),
    ];
  } else if (state === 'needs-input') {
    face = [
      rect(6, 11, 2, 2, C.eye),
      rect(6, 11, 1, 1, C.eyeShine),
      rect(7, 12, 1, 1, C.eyeShine),
      rect(10, 11, 2, 2, C.eye),
      rect(10, 11, 1, 1, C.eyeShine),
      rect(11, 12, 1, 1, C.eyeShine),
      rect(8, 14, 2, 1, C.darkGreen),
    ];
    arms = [
      rect(2, 10, 2, 3, C.leafGreen, 'wave-arm'),
      rect(13, 13, 2, 2, C.leafGreen),
    ];
    props = [
      rect(14, 6, 3, 4, C.amber, 'badge-q'),
      rect(15, 7, 1, 2, '#FFFFFF', 'badge-q'),
      rect(15, 10, 1, 1, '#FFFFFF', 'badge-q'),
    ];
  } else if (state === 'done') {
    face = [
      rect(5, 12, 1, 1, C.darkGreen),
      rect(6, 11, 2, 1, C.darkGreen),
      rect(9, 11, 2, 1, C.darkGreen),
      rect(11, 12, 1, 1, C.darkGreen),
      rect(7, 13, 4, 1, C.darkGreen),
      rect(8, 14, 2, 1, '#F43F5E'),
    ];
    arms = [
      rect(2, 9, 2, 3, C.leafGreen),
      rect(14, 9, 2, 3, C.leafGreen),
    ];
    props = [
      rect(2, 5, 2, 2, C.gold, 'star'),
      rect(14, 4, 2, 2, C.gold, 'star'),
    ];
  } else if (state === 'error') {
    face = [
      rect(5, 11, 1, 1, C.darkGreen),
      rect(6, 12, 1, 1, C.darkGreen),
      rect(5, 13, 1, 1, C.darkGreen),
      rect(11, 11, 1, 1, C.darkGreen),
      rect(10, 12, 1, 1, C.darkGreen),
      rect(11, 13, 1, 1, C.darkGreen),
      rect(7, 14, 4, 1, C.darkGreen),
    ];
    arms = [
      rect(3, 12, 2, 2, C.leafGreen),
      rect(13, 12, 2, 2, C.leafGreen),
    ];
    props = [
      rect(13, 8, 2, 3, C.blue, 'sweat-drop'),
      rect(14, 7, 1, 1, C.blue, 'sweat-drop'),
    ];
  } else if (state === 'sleeping') {
    face = [
      rect(5, 12, 3, 1, C.darkGreen),
      rect(10, 12, 3, 1, C.darkGreen),
      rect(8, 14, 2, 1, C.darkGreen),
    ];
    arms = [
      rect(3, 14, 2, 2, C.leafGreen),
      rect(13, 14, 2, 2, C.leafGreen),
    ];
    props = [
      `<text x="260" y="160" font-family="monospace" font-weight="bold" font-size="32" fill="#A78BFA" class="zzz-1">Z</text>`,
      `<text x="280" y="120" font-family="monospace" font-weight="bold" font-size="24" fill="#C4B5FD" class="zzz-2">z</text>`,
      `<text x="300" y="80" font-family="monospace" font-weight="bold" font-size="18" fill="#DDD6FE" class="zzz-3">z</text>`,
    ];
  } else if (state === 'music') {
    face = [
      rect(6, 12, 2, 2, C.eye),
      rect(6, 12, 1, 1, C.eyeShine),
      rect(10, 12, 2, 2, C.eye),
      rect(10, 12, 1, 1, C.eyeShine),
      rect(8, 14, 2, 1, C.darkGreen),
    ];
    arms = [
      rect(3, 13, 2, 2, C.leafGreen),
      rect(13, 13, 2, 2, C.leafGreen),
    ];
    props = [
      rect(2, 10, 2, 4, C.purple),
      rect(14, 10, 2, 4, C.purple),
      rect(3, 8, 2, 2, C.purple),
      rect(5, 7, 8, 1, C.purple),
      rect(13, 8, 2, 2, C.purple),
      rect(1, 11, 1, 2, C.gold),
      rect(16, 11, 1, 2, C.gold),
      `<text x="40" y="120" font-size="30" fill="#EC4899" class="note-1">&#9834;</text>`,
      `<text x="290" y="100" font-size="34" fill="#8B5CF6" class="note-2">&#9835;</text>`,
    ];
  } else {
    // idle
    face = [
      rect(6, 11, 2, 2, C.eye, 'eye-open'),
      rect(6, 11, 1, 1, C.eyeShine, 'eye-open'),
      rect(10, 11, 2, 2, C.eye, 'eye-open'),
      rect(10, 11, 1, 1, C.eyeShine, 'eye-open'),
      rect(6, 12, 2, 1, C.darkGreen, 'eye-closed'),
      rect(10, 12, 2, 1, C.darkGreen, 'eye-closed'),
      rect(8, 13, 2, 1, C.darkGreen),
    ];
    arms = [
      rect(3, 13, 2, 2, C.leafGreen),
      rect(13, 13, 2, 2, C.leafGreen),
    ];
  }

  const rootClass = state === 'done' ? 'cheer-body' :
                    state === 'working' ? 'idle-body' :
                    state === 'error' ? 'shake-body' :
                    state === 'music' ? 'music-body' :
                    state === 'needs-input' ? '' : 'idle-body';

  return `<svg xmlns="http://www.w3.org/2000/svg" aria-label="Fern mascot" viewBox="0 0 360 440" shape-rendering="crispEdges">
  <style>
    ${extraStyles}
  </style>
  <g class="${rootClass}">
    <!-- Sprout / Koru spiral -->
    ${sprout.join('\n    ')}
    <!-- Body -->
    ${body.join('\n    ')}
    <!-- Feet -->
    ${feet.join('\n    ')}
    <!-- Face -->
    ${face.join('\n    ')}
    <!-- Arms -->
    ${arms.join('\n    ')}
    <!-- Props & Overlays -->
    ${props.join('\n    ')}
  </g>
</svg>`;
}

const states = ['idle', 'working', 'needs-input', 'done', 'error', 'sleeping', 'music'];
states.forEach(st => {
  fs.writeFileSync(path.join(outDir, `${st}.svg`), createSvg(st, false), 'utf8');
  fs.writeFileSync(path.join(outDir, `${st}-sit.svg`), createSvg(st, true), 'utf8');
  console.log(`Generated ${st}.svg and ${st}-sit.svg`);
});
