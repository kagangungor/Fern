import type { DeviceArtKey } from "../devices/art";
import type { BudsBattery } from "../devices/connectedDevices";
import type { UsbIdentity } from "../devices/usbArtFor";

export interface BatteryDetails {
  present: boolean;
  percent: number | null;
  plugged_in: boolean;
  charging: boolean;
  seconds_left: number | null;
  seconds_to_full: number | null;
  health_percent: number | null;
  power_mode: string | null;
}

export interface WifiStatus {
  enabled: boolean;
  connected: boolean;
}

export interface MediaInfo {
  title: string;
  artist: string;
  is_playing: boolean;
  has_media: boolean;
  artwork?: string[];
  position_ms?: number;
  duration_ms?: number;
  seek_enabled?: boolean;
  position_updated_at?: number;
}

// A device that just connected or disconnected, shown briefly in the Dynamic Island
export interface DeviceAlert {
  id: string;
  art: DeviceArtKey;
  name: string;
  connected: boolean;
  battery: number | null;
  buds?: BudsBattery | null;
  usb?: boolean;
  runId: number;
}

export interface BluetoothConnection {
  id: string;
  name: string;
  kind: string;
  form: string | null;
  connected: boolean;
  battery: number | null;
}

export interface UsbConnection extends UsbIdentity {
  id: string;
  connected: boolean;
}
