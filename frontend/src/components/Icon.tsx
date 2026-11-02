/**
 * 图标登记表与统一图标组件。
 *
 * 全应用只允许通过 `<Icon name="..." />` 使用图标，不允许在业务组件里手写 SVG 路径。
 * 这样图标的默认尺寸、描边宽度与无障碍属性只有一个来源，替换图标库时也只需改这一处。
 */
import type { LucideProps } from "lucide-react";
import {
  Archive,
  ArrowDown,
  ArrowRight,
  ArrowUp,
  Bell,
  Briefcase,
  Calendar,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  CircleCheck,
  CircleX,
  Clock,
  Code,
  Command,
  Copy,
  CornerDownLeft,
  Cpu,
  Database,
  Download,
  Ellipsis,
  Expand,
  ExternalLink,
  Eye,
  EyeOff,
  File,
  FileCode,
  FilePlus,
  FileText,
  Filter,
  Folder,
  FolderOpen,
  FolderPlus,
  GitBranch,
  GitCompare,
  Globe,
  HardDrive,
  Hash,
  Image as ImageIcon,
  Info,
  KeyRound,
  Layers,
  ListTodo,
  LoaderCircle,
  Lock,
  Mail,
  MessageSquare,
  Minus,
  Monitor,
  Moon,
  PanelLeft,
  PanelRight,
  PanelRightClose,
  PanelRightOpen,
  Paperclip,
  Play,
  Plus,
  Quote,
  RefreshCw,
  RotateCcw,
  Save,
  Search,
  Settings,
  Shield,
  ShieldCheck,
  Shrink,
  SlidersVertical,
  Sparkles,
  Square,
  SquareCheckBig,
  SquarePen,
  Sun,
  Table,
  Terminal,
  Trash2,
  TriangleAlert,
  Type,
  Undo2,
  User,
  X,
  Zap,
} from "lucide-react";

/**
 * 图标登记表：把原型里使用的图标名映射到 lucide 组件。
 *
 * 原型里 81 个 SVG symbol 的对应 lucide 组件。
 *
 * 这里只登记实际用到的图标，Vite 会按需 tree-shake。
 * 新增图标时直接在此表追加，不要在组件里手写 path。
 * 键就是业务代码里书写的 `name`，因此同一个图标可以登记多个别名
 * （例如 `git-branch` 与 `branch` 都指向 GitBranch）。
 */
export const icons = {
  code: Code,
  briefcase: Briefcase,
  "chevron-down": ChevronDown,
  "chevron-right": ChevronRight,
  "chevron-left": ChevronLeft,
  "chevron-up": ChevronUp,
  minus: Minus,
  type: Type,
  "file-plus": FilePlus,
  plus: Plus,
  search: Search,
  settings: Settings,
  command: Command,
  folder: Folder,
  "folder-open": FolderOpen,
  "folder-plus": FolderPlus,
  file: File,
  "file-code": FileCode,
  "file-text": FileText,
  "git-branch": GitBranch,
  branch: GitBranch,
  diff: GitCompare,
  terminal: Terminal,
  check: Check,
  "check-circle": CircleCheck,
  x: X,
  "x-circle": CircleX,
  alert: TriangleAlert,
  info: Info,
  stop: Square,
  "arrow-up": ArrowUp,
  "arrow-down": ArrowDown,
  "arrow-right": ArrowRight,
  "corner-down-left": CornerDownLeft,
  paperclip: Paperclip,
  image: ImageIcon,
  copy: Copy,
  refresh: RefreshCw,
  more: Ellipsis,
  shield: Shield,
  lock: Lock,
  key: KeyRound,
  eye: Eye,
  "eye-off": EyeOff,
  edit: SquarePen,
  save: Save,
  trash: Trash2,
  archive: Archive,
  download: Download,
  "external-link": ExternalLink,
  play: Play,
  clock: Clock,
  sparkles: Sparkles,
  "list-todo": ListTodo,
  user: User,
  layers: Layers,
  sun: Sun,
  moon: Moon,
  monitor: Monitor,
  sidebar: PanelLeft,
  panel: PanelRight,
  "panel-close": PanelRightClose,
  "panel-open": PanelRightOpen,
  maximize: Expand,
  minimize: Shrink,
  hash: Hash,
  quote: Quote,
  "shield-check": ShieldCheck,
  "check-square": SquareCheckBig,
  "list-filter": Filter,
  table: Table,
  globe: Globe,
  bell: Bell,
  database: Database,
  cpu: Cpu,
  zap: Zap,
  sliders: SlidersVertical,
  mail: Mail,
  calendar: Calendar,
  message: MessageSquare,
  filter: Filter,
  "rotate-ccw": RotateCcw,
  loader: LoaderCircle,
  "hard-drive": HardDrive,
  undo: Undo2,
} as const;

/** 图标名称的联合类型，直接由登记表的键推导，保证 `name` 不会写错。 */
export type IconName = keyof typeof icons;

/**
 * 图标组件的属性。
 *
 * 在 lucide 原生属性基础上收紧了尺寸与描边：统一由 `size` 与 `strokeWidth` 控制，
 * 其余属性（className、style、onClick 等）原样透传。
 */
export interface IconProps extends Omit<LucideProps, "size" | "strokeWidth"> {
  /** 登记表里的图标名。 */
  name: IconName;
  /** 图标边长（像素），默认 16。 */
  size?: number;
  /** 描边宽度，默认 1.75。 */
  strokeWidth?: number;
}

/**
 * 统一图标入口。
 *
 * 负责：按名称取出图标组件，套上默认尺寸与描边宽度。
 * props：`name` 必填，`size` 默认 16px，`strokeWidth` 默认 1.75，其余属性透传。
 * 关键约定：颜色跟随 `currentColor`；`aria-hidden` 固定为真，因为图标只是视觉辅助，
 * 文字说明由使用方提供，避免读屏重复朗读。
 */
export function Icon({ name, size = 16, strokeWidth = 1.75, ...rest }: IconProps) {
  const Component = icons[name];
  return <Component size={size} strokeWidth={strokeWidth} aria-hidden {...rest} />;
}
