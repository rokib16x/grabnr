import {
  AppWindow,
  CaretRight,
  Disc,
  File,
  FileArchive,
  FileText,
  FilmStrip,
  ImageSquare,
  MusicNotes,
  Check,
  Clock,
  Copy,
  DeviceMobile,
  DotsThree,
  DownloadSimple,
  Folder,
  Lightning,
  LinkSimple,
  MagnifyingGlass,
  Network,
  Pause,
  Play,
  Plus,
  Power,
  SlidersHorizontal,
  Trash,
  Tray,
  Usb,
  WarningCircle,
  WifiHigh,
  X,
  type Icon as PhosphorIcon,
  type IconWeight,
} from "@phosphor-icons/react";

// Phosphor (https://phosphoricons.com): one consistent outline family, 24px grid, round joins.
const I: Record<string, PhosphorIcon> = {
  plus: Plus,
  pause: Pause,
  play: Play,
  trash: Trash,
  more: DotsThree,
  search: MagnifyingGlass,
  link: LinkSimple,
  wifi: WifiHigh,
  ethernet: Network,
  phone: DeviceMobile,
  usb: Usb,
  sliders: SlidersHorizontal,
  check: Check,
  folder: Folder,
  download: DownloadSimple,
  inbox: Tray,
  clock: Clock,
  bolt: Lightning,
  alert: WarningCircle,
  x: X,
  power: Power,
  copy: Copy,
  chevron: CaretRight,
  video: FilmStrip,
  audio: MusicNotes,
  image: ImageSquare,
  document: FileText,
  archive: FileArchive,
  disk: Disc,
  app: AppWindow,
  file: File,
};
// Media controls read better solid; everything else uses the bold outline so it holds up at 12-16px.
const FILLED = new Set(["play", "pause"]);

export function Icon({ name, size = 16, className, weight }: { name: string; size?: number; className?: string; weight?: IconWeight }) {
  const C = I[name] ?? LinkSimple;
  return <C className={className} size={size} weight={weight ?? (FILLED.has(name) ? "fill" : "bold")} aria-hidden="true" focusable="false" />;
}

export const linkIcon = (kind: string) => (kind === "wi_fi" ? "wifi" : kind === "ethernet" ? "ethernet" : kind === "cellular" ? "phone" : "usb");
