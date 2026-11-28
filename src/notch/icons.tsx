import {
  Bluetooth as BluetoothLucide,
  Settings as SettingsLucide,
  Moon,
  PanelBottom,
  PanelTop,
  Bell,
  RotateCw,
  BatteryCharging,
} from "lucide-react";

// Simple SVG icons
export function WifiIcon({ enabled, connected }: { enabled: boolean; connected: boolean }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" opacity={!enabled ? 0.4 : connected ? 1 : 0.7}>
      <path d="M5 12.55a11 11 0 0 1 14.08 0" />
      <path d="M1.42 9a16 16 0 0 1 21.16 0" />
      <path d="M8.53 16.11a6 6 0 0 1 6.95 0" />
      <line x1="12" y1="20" x2="12.01" y2="20" />
    </svg>
  );
}

export function EthernetIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <rect x="4" y="4" width="16" height="11" rx="2" />
      <path d="M8 15v3h8v-3" />
      <line x1="9" y1="8" x2="9" y2="11" />
      <line x1="12" y1="8" x2="12" y2="11" />
      <line x1="15" y1="8" x2="15" y2="11" />
    </svg>
  );
}

export function TrayIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="6" height="6" rx="1" />
      <rect x="15" y="3" width="6" height="6" rx="1" />
      <rect x="15" y="15" width="6" height="6" rx="1" />
      <rect x="3" y="15" width="6" height="6" rx="1" />
    </svg>
  );
}

export function BatteryIcon({ charging, level, threshold = 20 }: { charging: boolean; level: number; threshold?: number }) {
  const percentage = Math.min(Math.max(level, 0), 100);

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      position: 'relative',
      height: '14px',
      justifyContent: 'center'
    }}>
      <svg width="20" height="10" viewBox="0 0 20 10" fill="none">
        {/* Battery Shell - Centered at 9px within 20px width, ignoring the tip's offset */}
        <rect
          x="2" y="0.75" width="14" height="8.5" rx="2.4"
          stroke="currentColor" strokeOpacity={0.35} strokeWidth="1.1"
        />
        {/* Battery Tip */}
        <path
          d="M17.5 3.5V6.5"
          stroke="currentColor" strokeOpacity={0.35} strokeWidth="1.2" strokeLinecap="round"
        />
        {/* Fill */}
        <rect
          x="3.8" y="2.5"
          width={Math.max(0.5, (percentage / 100) * 10.4)}
          height="5" rx="1"
          fill={charging ? "#32D74B" : (percentage <= threshold ? "#FF453A" : "white")}
        />
      </svg>
      {/* Charging Bolt - Centered on the battery body */}
      {charging && (
        <div style={{
          position: 'absolute',
          top: '50%',
          left: '9px',
          transform: 'translate(-50%, -50%)',
          color: 'white',
          filter: 'drop-shadow(0px 0px 1.5px rgba(0,0,0,0.8))'
        }}>
          <svg width="7" height="10" viewBox="0 0 8 12" fill="currentColor">
            <path d="M4.5 0L0 7H3.5L2.5 12L8 5H4.5L5.5 0H4.5Z" />
          </svg>
        </div>
      )}
    </div>
  );
}

export function GreenDownArrowIcon() {
  return (
    <div style={{
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      width: '18px',
      height: '18px',
      borderRadius: '50%',
      background: 'rgba(50, 215, 75, 0.15)',
      border: '1px solid rgba(50, 215, 75, 0.35)',
      boxShadow: '0 0 8px rgba(50, 215, 75, 0.25)',
      flexShrink: 0,
      verticalAlign: 'middle'
    }}>
      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="#32D74B" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
        <line x1="12" y1="4" x2="12" y2="16"></line>
        <polyline points="18 10 12 16 6 10"></polyline>
        <line x1="6" y1="20" x2="18" y2="20"></line>
      </svg>
    </div>
  );
}

export function BluetoothIcon() {
  return <BluetoothLucide size={16} strokeWidth={2} />;
}

export function SettingsIcon() {
  return <SettingsLucide size={16} strokeWidth={2} />;
}

export function BrightnessIcon({ level }: { level: number }) {
  const rays = [
    [12, 2.5, 12, 5],
    [12, 19, 12, 21.5],
    [2.5, 12, 5, 12],
    [19, 12, 21.5, 12],
    [5.3, 5.3, 7, 7],
    [17, 17, 18.7, 18.7],
    [5.3, 18.7, 7, 17],
    [17, 7, 18.7, 5.3],
  ];
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ opacity: 0.5 }}>
      <circle cx="12" cy="12" r="4.2" fill="currentColor" />
      <g opacity={0.35 + (Math.min(100, Math.max(0, level)) / 100) * 0.65}>
        {rays.map(([x1, y1, x2, y2], i) => (
          <line key={i} x1={x1} y1={y1} x2={x2} y2={y2} />
        ))}
      </g>
    </svg>
  );
}

export function MoonIcon() {
  return <Moon size={16} strokeWidth={2} />;
}

export function DockIcon() {
  return <PanelBottom size={16} strokeWidth={2} />;
}

export function NotchIcon() {
  return <PanelTop size={16} strokeWidth={2} />;
}

export function BellIcon() {
  return <Bell size={16} strokeWidth={2} />;
}

export function ReloadIcon() {
  return <RotateCw size={16} strokeWidth={2} />;
}

export function BatterySaverIcon() {
  return <BatteryCharging size={16} strokeWidth={2} />;
}
