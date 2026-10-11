// Small inline icons (24x24, stroke = currentColor).
import type { JSX } from "solid-js";

const Icon = (props: { children: JSX.Element; title?: string }) => (
  <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8"
    stroke-linecap="round" stroke-linejoin="round" aria-hidden={props.title ? undefined : "true"} role={props.title ? "img" : undefined}>
    {props.title ? <title>{props.title}</title> : null}
    {props.children}
  </svg>
);

export const UndoIcon = () => <Icon><path d="M9 14 4 9l5-5" /><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" /></Icon>;
export const RedoIcon = () => <Icon><path d="m15 14 5-5-5-5" /><path d="M20 9H9.5a5.5 5.5 0 0 0 0 11H13" /></Icon>;
export const EraserIcon = () => <Icon><path d="m7 21-4.3-4.3a1 1 0 0 1 0-1.4l9.6-9.6a1 1 0 0 1 1.4 0l5.6 5.6a1 1 0 0 1 0 1.4L13 21" /><path d="M22 21H7" /><path d="m5 11 9 9" /></Icon>;
export const PlusIcon = () => <Icon><path d="M12 5v14M5 12h14" /></Icon>;
export const CloseIcon = () => <Icon><path d="M18 6 6 18M6 6l12 12" /></Icon>;
export const PencilIcon = () => <Icon><path d="M17 3a2.85 2.85 0 0 1 4 4L7.5 20.5 2 22l1.5-5.5Z" /></Icon>;
export const CommentIcon = () => <Icon><path d="M21 12a8 8 0 0 1-11.6 7.1L4 20.5l1.4-4.9A8 8 0 1 1 21 12Z" /></Icon>;
export const TrashIcon = () => <Icon><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3" /></Icon>;
export const PrevIcon = () => <Icon><path d="m15 18-6-6 6-6" /></Icon>;
export const NextIcon = () => <Icon><path d="m9 18 6-6-6-6" /></Icon>;
export const HelpIcon = () => <Icon><circle cx="12" cy="12" r="9.5" /><path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3" /><path d="M12 17h.01" /></Icon>;
export const BrushIcon = () => (
  <Icon>
    <path d="m14.6 9.4 5.9-5.9a1.8 1.8 0 0 0-2.5-2.5l-5.9 5.9" />
    <path d="m9.6 10.4 4 4" />
    <path d="M7.5 13c-2 0-3.5 1.6-3.5 3.5 0 1.9-1 2.9-2 3.5 4 1 8.5.4 8.5-3.5 0-2-1.4-3.5-3-3.5Z" />
  </Icon>
);
export const TagsIcon = () => (
  <Icon>
    <path d="M3 7h13M3 12h13M3 17h9" />
    <circle cx="20" cy="7" r="1" />
    <circle cx="20" cy="12" r="1" />
  </Icon>
);
export const PinIcon = (props: { filled?: boolean }) => (
  <Icon>
    <path d="M12 17v5" />
    <path d="M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.2V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.8a2 2 0 0 0-1.1-1.8l-1.8-.9A2 2 0 0 1 15 10.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" fill={props.filled ? "currentColor" : "none"} />
  </Icon>
);
export const SearchIcon = () => (
  <Icon>
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-3.5-3.5" />
  </Icon>
);
