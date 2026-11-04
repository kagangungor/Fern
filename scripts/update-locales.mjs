import fs from "fs";

const enAdditions = {
  "appearance.glassWallpaper": "Show wallpaper in glass",
  "appearance.glassWallpaper.desc": "Blend desktop wallpaper behind glass surfaces for depth",
  "notch.agent.group": "AI Agents",
  "notch.agent.enable": "Claude Code & AI agents",
  "notch.agent.enable.desc": "Show agent status, progress, questions and allow actions in the notch",
  "notch.agent.sounds": "Sound effects",
  "notch.agent.sounds.desc": "Play sounds when agents finish, fail or request input",
  "notch.agent.sound.needs_input": "Needs input",
  "notch.agent.sound.done": "Done",
  "notch.agent.sound.error": "Error",
  "notch.agent.sound.choose": "Choose sound…",
  "notch.agent.sound.change": "Change sound…",
  "notch.agent.sound.reset": "Reset to default",
  "notch.agent.sound.test": "Test sound",
  "notch.agent.sound.defaultActive": "Using built-in sound",
  "notch.agent.sound.customActive": "Using custom sound file",
  "notch.edgeDelay": "Hover delay",
  "notch.edgeDelay.desc": "Delay before opening the notch on edge hover",
  "overlays.volumeSide": "Volume side",
  "overlays.volumeSide.desc": "Screen edge for the volume slider card",
  "overlays.brightnessSide": "Brightness side",
  "overlays.brightnessSide.desc": "Screen edge for the brightness slider card",
  "overlays.edgeDelay": "Hover delay",
  "overlays.edgeDelay.desc": "Delay before opening sliders on edge hover",
  "common.side.left": "Left",
  "common.side.right": "Right",
  "dock.attentionDots": "Notification dots",
  "dock.attentionDots.desc": "Show badge dots or unread count on apps requesting attention",
  "dock.hideDelay": "Hide delay",
  "dock.hideDelay.desc": "Delay before the dock hides after cursor leaves",
};

const trAdditions = {
  "appearance.glassWallpaper": "Cam efektinde masaüstü duvar kağıdı",
  "appearance.glassWallpaper.desc": "Derinlik hissi için cam yüzeylerin arkasında masaüstü duvar kağıdını harmanla",
  "notch.agent.group": "Yapay Zeka Ajanları",
  "notch.agent.enable": "Claude Code & Yapay zeka ajanları",
  "notch.agent.enable.desc": "Ajan durumunu, ilerlemesini, sorularını ve onay eylemlerini çentikte göster",
  "notch.agent.sounds": "Ses efektleri",
  "notch.agent.sounds.desc": "Ajanlar tamamlandığında, hata verdiğinde veya girdi beklediğinde ses çal",
  "notch.agent.sound.needs_input": "Girdi bekliyor",
  "notch.agent.sound.done": "Tamamlandı",
  "notch.agent.sound.error": "Hata",
  "notch.agent.sound.choose": "Ses seç…",
  "notch.agent.sound.change": "Sesi değiştir…",
  "notch.agent.sound.reset": "Varsayılana sıfırla",
  "notch.agent.sound.test": "Sesi test et",
  "notch.agent.sound.defaultActive": "Dahili ses kullanılıyor",
  "notch.agent.sound.customActive": "Özel ses dosyası kullanılıyor",
  "notch.edgeDelay": "Gecikme süresi",
  "notch.edgeDelay.desc": "Fareyle kenara gelindiğinde çentiğin açılma gecikmesi",
  "overlays.volumeSide": "Ses düzeyi tarafı",
  "overlays.volumeSide.desc": "Ses kaydırıcı kartının gösterileceği ekran kenarı",
  "overlays.brightnessSide": "Parlaklık tarafı",
  "overlays.brightnessSide.desc": "Parlaklık kaydırıcı kartının gösterileceği ekran kenarı",
  "overlays.edgeDelay": "Gecikme süresi",
  "overlays.edgeDelay.desc": "Fare kenara getirildiğinde kaydırıcının açılma gecikmesi",
  "common.side.left": "Sol",
  "common.side.right": "Sağ",
  "dock.attentionDots": "Bildirim noktaları",
  "dock.attentionDots.desc": "Dikkat gerektiren uygulamalarda rozet veya okunmamış bildirim sayısı göster",
  "dock.hideDelay": "Gizlenme gecikmesi",
  "dock.hideDelay.desc": "İmleç ayrıldıktan sonra dock'un gizlenme gecikmesi",
};

function updateFile(path, additions) {
  const content = JSON.parse(fs.readFileSync(path, "utf8"));
  Object.assign(content, additions);
  fs.writeFileSync(path, JSON.stringify(content, null, 2) + "\n", "utf8");
  console.log(`Updated ${path} (+${Object.keys(additions).length} keys)`);
}

updateFile("src/locales/en.json", enAdditions);
updateFile("src/locales/tr.json", trAdditions);
