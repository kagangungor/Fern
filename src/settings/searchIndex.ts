import type { SettingsTab } from "./types";

export interface SearchableSetting {
  id: string;
  tab: SettingsTab;
  sectionId?: string;
  titleKey: string;
  descKey?: string;
  keywords: string[];
}

export const SETTINGS_SEARCH_INDEX: SearchableSetting[] = [
  // General Tab
  {
    id: "general-autostart",
    tab: "general",
    sectionId: "general-system",
    titleKey: "general.launchAtLogin",
    descKey: "general.launchAtLogin.desc",
    keywords: ["autostart", "başlangıç", "açılış", "startup", "giriş", "windows"],
  },
  {
    id: "general-update-indicator",
    tab: "general",
    sectionId: "general-system",
    titleKey: "general.updateIndicator",
    descKey: "general.updateIndicator.desc",
    keywords: ["update", "güncelleme", "bildirim", "indicator", "versiyon"],
  },
  {
    id: "general-language",
    tab: "general",
    sectionId: "general-system",
    titleKey: "general.language",
    descKey: "general.language.desc",
    keywords: ["language", "dil", "türkçe", "english", "locale", "lisan"],
  },
  {
    id: "general-week-start",
    tab: "general",
    sectionId: "general-system",
    titleKey: "general.weekStart",
    descKey: "general.weekStart.desc",
    keywords: ["week", "hafta", "pazartesi", "pazar", "monday", "sunday", "takvim"],
  },
  {
    id: "general-time-format",
    tab: "general",
    sectionId: "general-system",
    titleKey: "general.time24h",
    descKey: "general.time24h.desc",
    keywords: ["time", "saat", "24h", "12h", "format", "zaman"],
  },
  {
    id: "general-memory-saver",
    tab: "general",
    sectionId: "general-system",
    titleKey: "general.memorySaver",
    descKey: "general.memorySaver.desc",
    keywords: ["memory", "ram", "bellek", "tasarruf", "saver"],
  },
  {
    id: "general-low-battery",
    tab: "general",
    sectionId: "general-system",
    titleKey: "general.lowBattery",
    descKey: "general.lowBattery.desc",
    keywords: ["battery", "pil", "şarj", "uyarı", "low battery", "eşik"],
  },
  {
    id: "general-restart",
    tab: "general",
    sectionId: "general-app",
    titleKey: "general.restart",
    descKey: "general.restart.desc",
    keywords: ["restart", "yeniden başlat", "reboot"],
  },
  {
    id: "general-quit",
    tab: "general",
    sectionId: "general-app",
    titleKey: "general.quit",
    descKey: "general.quit.desc",
    keywords: ["quit", "çıkış", "kapat", "exit"],
  },

  // Appearance Tab
  {
    id: "appearance-style",
    tab: "appearance",
    sectionId: "appearance-theme",
    titleKey: "appearance.theme.group",
    keywords: ["theme", "tema", "karanlık", "aydınlık", "light", "dark", "glass", "frosted", "liquid", "cam"],
  },
  {
    id: "appearance-accent",
    tab: "appearance",
    sectionId: "appearance-theme",
    titleKey: "appearance.accent",
    descKey: "appearance.accent.desc",
    keywords: ["accent", "vurgu", "renk", "color", "windows accent", "blue", "mor", "kırmızı"],
  },
  {
    id: "appearance-scale",
    tab: "appearance",
    sectionId: "appearance-theme",
    titleKey: "appearance.scale",
    descKey: "appearance.scale.desc",
    keywords: ["scale", "ölçek", "boyut", "zoom", "size", "büyüt", "küçült"],
  },
  {
    id: "appearance-corners",
    tab: "appearance",
    sectionId: "appearance-display",
    titleKey: "appearance.corners",
    descKey: "appearance.corners.desc",
    keywords: ["corners", "köşeler", "yuvarlak", "rounded", "ekran"],
  },

  // Notch Tab
  {
    id: "notch-mode",
    tab: "notch",
    sectionId: "notch-display",
    titleKey: "notch.mode",
    descKey: "notch.mode.desc",
    keywords: ["notch", "çentik", "ada", "island", "smart", "fixed", "peek", "otomatik gizle"],
  },
  {
    id: "notch-panel-cc",
    tab: "notch",
    sectionId: "notch-panels",
    titleKey: "notch.panelControlCenter",
    descKey: "notch.panelControlCenter.desc",
    keywords: ["control center", "denetim merkezi", "cc", "panel", "ses", "parlaklık"],
  },
  {
    id: "notch-panel-shelf",
    tab: "notch",
    sectionId: "notch-panels",
    titleKey: "notch.panelShelf",
    descKey: "notch.panelShelf.desc",
    keywords: ["shelf", "raf", "dosya", "drop", "sürükle", "clipboard"],
  },
  {
    id: "notch-panel-status",
    tab: "notch",
    sectionId: "notch-panels",
    titleKey: "notch.panelStatus",
    descKey: "notch.panelStatus.desc",
    keywords: ["status", "durum", "panel", "widget", "pil", "hava"],
  },
  {
    id: "notch-scroll-direction",
    tab: "notch",
    sectionId: "notch-panels",
    titleKey: "notch.scrollDirection",
    descKey: "notch.scrollDirection.desc",
    keywords: ["scroll", "kaydırma", "tekerlek", "wheel", "direction", "yön", "doğal", "ters", "natural", "inverted"],
  },
  {
    id: "notch-panel-order",
    tab: "notch",
    sectionId: "notch-panels",
    titleKey: "notch.panelOrder",
    descKey: "notch.panelOrder.desc",
    keywords: ["order", "sıra", "reorder", "sıralama", "paneller"],
  },
  {
    id: "notch-music",
    tab: "notch",
    sectionId: "notch-music",
    titleKey: "notch.music",
    descKey: "notch.music.desc",
    keywords: ["music", "müzik", "spotify", "şarkı", "album", "parıltı", "glow", "ambience"],
  },
  {
    id: "notch-weather",
    tab: "notch",
    sectionId: "notch-widgets",
    titleKey: "notch.weather",
    descKey: "notch.weather.desc",
    keywords: ["weather", "hava", "durumu", "sıcaklık", "şehir", "city"],
  },

  // Dock Tab
  {
    id: "dock-mode",
    tab: "dock",
    sectionId: "dock-main",
    titleKey: "dock.mode",
    descKey: "dock.mode.desc",
    keywords: ["dock", "görev çubuğu", "taskbar", "smart", "fixed", "peek", "gizle"],
  },
  {
    id: "dock-preview",
    tab: "dock",
    sectionId: "dock-shortcuts",
    titleKey: "dock.previews",
    descKey: "dock.previews.desc",
    keywords: ["preview", "önizleme", "thumbnail", "canlı", "hover", "pencere"],
  },
  {
    id: "dock-indicator",
    tab: "dock",
    sectionId: "dock-style",
    titleKey: "dock.indicator",
    descKey: "dock.indicator.desc",
    keywords: ["indicator", "gösterge", "çizgi", "nokta", "noktalar", "dots", "bar", "çalışan"],
  },
  {
    id: "dock-show-end-task",
    tab: "dock",
    sectionId: "dock-style",
    titleKey: "dock.showEndTask",
    descKey: "dock.showEndTask.desc",
    keywords: ["end task", "görevi sonlandır", "sonlandır", "görev", "kapat", "kill", "force close", "terminate"],
  },
  {
    id: "dock-icon-only",
    tab: "dock",
    sectionId: "dock-style",
    titleKey: "dock.iconOnly",
    descKey: "dock.iconOnly.desc",
    keywords: ["icon only", "sadece simge", "minimal"],
  },
  {
    id: "dock-start-icon",
    tab: "dock",
    sectionId: "dock-style",
    titleKey: "dock.startIcon",
    descKey: "dock.startIcon.desc",
    keywords: ["start icon", "başlat", "logo", "fern", "windows"],
  },
  {
    id: "dock-menu-items",
    tab: "dock",
    sectionId: "dock-menu-items",
    titleKey: "dock.section.menuItems",
    descKey: "dock.section.menuItems.desc",
    keywords: ["menu items", "sağ tık", "menü", "öğeler", "seçenekler", "options"],
  },
  {
    id: "dock-hidden-apps",
    tab: "dock",
    sectionId: "dock-hidden-apps",
    titleKey: "dock.hiddenApps",
    descKey: "dock.hiddenApps.desc",
    keywords: ["hidden", "gizli", "gizlenen", "apps", "unhide"],
  },

  // Overlays Tab
  {
    id: "overlays-volume",
    tab: "overlays",
    sectionId: "overlays-main",
    titleKey: "overlays.volume",
    descKey: "overlays.volume.desc",
    keywords: ["volume", "ses", "overlay", "hud", "gösterge"],
  },
  {
    id: "overlays-brightness",
    tab: "overlays",
    sectionId: "overlays-main",
    titleKey: "overlays.brightness",
    descKey: "overlays.brightness.desc",
    keywords: ["brightness", "parlaklık", "overlay", "hud"],
  },

  // Diagnostics Tab
  {
    id: "diagnostics-copy",
    tab: "diagnostics",
    sectionId: "diagnostics-system",
    titleKey: "about.diagnostics.copy",
    descKey: "about.diagnostics.desc",
    keywords: ["diagnostics", "tanılama", "hata", "rapor", "report", "kopyala"],
  },
  {
    id: "diagnostics-logs",
    tab: "diagnostics",
    sectionId: "diagnostics-system",
    titleKey: "about.logs",
    descKey: "about.logs.desc",
    keywords: ["logs", "günlük", "loglar", "klasör", "folder"],
  },
  {
    id: "diagnostics-icons",
    tab: "diagnostics",
    sectionId: "diagnostics-maintenance",
    titleKey: "about.icons",
    descKey: "about.icons.desc",
    keywords: ["clear icons", "önbellek", "simge", "cache", "yenile"],
  },

  // About Tab
  {
    id: "about-version",
    tab: "about",
    titleKey: "about.update.check",
    keywords: ["version", "sürüm", "güncelleme", "update"],
  },
  {
    id: "about-backup",
    tab: "about",
    titleKey: "about.export",
    descKey: "about.export.desc",
    keywords: ["backup", "yedek", "dışa aktar", "içe aktar", "export", "import"],
  },
  {
    id: "about-reset",
    tab: "about",
    titleKey: "about.reset",
    descKey: "about.reset.desc",
    keywords: ["reset", "sıfırla", "varsayılan"],
  },
];
