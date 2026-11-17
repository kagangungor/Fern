import {
  File,
  FileArchive,
  FileAudio,
  FileCode2,
  FileSpreadsheet,
  FileText,
  FileVideo,
  Presentation,
  Type,
} from "lucide-react";

type Props = {
  kind: "file" | "text";
  name: string;
};

type Category = {
  icon: typeof File;
  color: string;
};

const EXT_CATEGORY: Record<string, Category> = {
  pdf: { icon: FileText, color: "#ef4444" },
  doc: { icon: FileText, color: "#3b82f6" },
  docx: { icon: FileText, color: "#3b82f6" },
  odt: { icon: FileText, color: "#3b82f6" },
  rtf: { icon: FileText, color: "#3b82f6" },
  xls: { icon: FileSpreadsheet, color: "#22c55e" },
  xlsx: { icon: FileSpreadsheet, color: "#22c55e" },
  ods: { icon: FileSpreadsheet, color: "#22c55e" },
  csv: { icon: FileSpreadsheet, color: "#22c55e" },
  ppt: { icon: Presentation, color: "#f97316" },
  pptx: { icon: Presentation, color: "#f97316" },
  odp: { icon: Presentation, color: "#f97316" },
  zip: { icon: FileArchive, color: "#eab308" },
  rar: { icon: FileArchive, color: "#eab308" },
  "7z": { icon: FileArchive, color: "#eab308" },
  tar: { icon: FileArchive, color: "#eab308" },
  gz: { icon: FileArchive, color: "#eab308" },
  mp3: { icon: FileAudio, color: "#a855f7" },
  wav: { icon: FileAudio, color: "#a855f7" },
  flac: { icon: FileAudio, color: "#a855f7" },
  ogg: { icon: FileAudio, color: "#a855f7" },
  m4a: { icon: FileAudio, color: "#a855f7" },
  mp4: { icon: FileVideo, color: "#ec4899" },
  mkv: { icon: FileVideo, color: "#ec4899" },
  mov: { icon: FileVideo, color: "#ec4899" },
  avi: { icon: FileVideo, color: "#ec4899" },
  webm: { icon: FileVideo, color: "#ec4899" },
  ts: { icon: FileCode2, color: "#38bdf8" },
  tsx: { icon: FileCode2, color: "#38bdf8" },
  js: { icon: FileCode2, color: "#facc15" },
  jsx: { icon: FileCode2, color: "#facc15" },
  rs: { icon: FileCode2, color: "#fb923c" },
  py: { icon: FileCode2, color: "#60a5fa" },
  json: { icon: FileCode2, color: "#a3e635" },
  html: { icon: FileCode2, color: "#f87171" },
  css: { icon: FileCode2, color: "#38bdf8" },
  md: { icon: FileText, color: "#cbd5e1" },
  txt: { icon: FileText, color: "#cbd5e1" },
};

export function ShelfTypeIcon({ kind, name }: Props) {
  if (kind === "text") {
    return <Type size={22} strokeWidth={1.8} style={{ color: "rgba(255, 255, 255, 0.78)" }} />;
  }
  const dot = name.lastIndexOf(".");
  const ext = dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
  const match = EXT_CATEGORY[ext];
  const Icon = match?.icon ?? File;
  const color = match?.color ?? "rgba(255, 255, 255, 0.72)";
  return <Icon size={22} strokeWidth={1.75} style={{ color }} />;
}
