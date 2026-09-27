// Small line icons for the sidebar and the setting tiles, drawn on a 16x16 grid in the style of SF Symbols.

import mark from "./assets/mark.png";

type P = { size?: number };

function Icon({ size = 14, children }: P & { children: React.ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      {children}
    </svg>
  );
}

export const BrandMark = ({ size = 20 }: P) => <img className="brand-mark" src={mark} width={size} height={size} alt="" />;

export const Search = () => (
  <Icon size={13}>
    <circle cx="7" cy="7" r="4.6" />
    <path d="M10.4 10.4L14 14" />
  </Icon>
);

export const Gauge = (p: P) => (
  <Icon {...p}>
    <path d="M2.5 11.5a5.5 5.5 0 1 1 11 0" />
    <path d="M8 11.5l2.6-3.4" />
  </Icon>
);

export const Bell = (p: P) => (
  <Icon {...p}>
    <path d="M4 11V7.2a4 4 0 0 1 8 0V11l1.2 1.3H2.8z" />
    <path d="M6.6 14h2.8" />
  </Icon>
);

export const Terminal = (p: P) => (
  <Icon {...p}>
    <rect x="1.8" y="2.8" width="12.4" height="10.4" rx="2" />
    <path d="M4.6 6.4L6.8 8.2 4.6 10M8.6 10.2h2.8" />
  </Icon>
);

export const Hook = (p: P) => (
  <Icon {...p}>
    <path d="M8 1.8v7.4a3 3 0 1 1-6 0" />
    <path d="M8 4.2h3.6" />
  </Icon>
);

export const MenuBar = (p: P) => (
  <Icon {...p}>
    <rect x="1.8" y="2.8" width="12.4" height="10.4" rx="2" />
    <path d="M1.8 6h12.4" />
  </Icon>
);

export const Calendar = (p: P) => (
  <Icon {...p}>
    <rect x="2" y="3" width="12" height="11" rx="2" />
    <path d="M2 6.6h12M5.2 1.8v2.4M10.8 1.8v2.4" />
  </Icon>
);

export const Power = (p: P) => (
  <Icon {...p}>
    <path d="M8 2v5.4" />
    <path d="M4.6 4.2a5 5 0 1 0 6.8 0" />
  </Icon>
);

export const Clock = (p: P) => (
  <Icon {...p}>
    <circle cx="8" cy="8" r="6" />
    <path d="M8 4.6V8l2.4 1.6" />
  </Icon>
);

export const Key = (p: P) => (
  <Icon {...p}>
    <circle cx="5.2" cy="10.8" r="3" />
    <path d="M7.4 8.6L13.6 2.4M11.6 4.4l1.6 1.6M10 6l1.2 1.2" />
  </Icon>
);

export const Download = (p: P) => (
  <Icon {...p}>
    <path d="M8 2v8M4.8 7L8 10.2 11.2 7M2.6 13.6h10.8" />
  </Icon>
);

export const Info = (p: P) => (
  <Icon {...p}>
    <circle cx="8" cy="8" r="6" />
    <path d="M8 7.2V11" />
    <circle cx="8" cy="4.9" r="0.4" fill="currentColor" />
  </Icon>
);

export const Chat = (p: P) => (
  <Icon {...p}>
    <path d="M2.4 4.4a2 2 0 0 1 2-2h7.2a2 2 0 0 1 2 2v4.4a2 2 0 0 1-2 2H7l-3 2.6v-2.6h0.4a2 2 0 0 1-2-2z" />
  </Icon>
);

export const Shield = (p: P) => (
  <Icon {...p}>
    <path d="M8 1.8l5 1.9v3.8c0 3.2-2.2 5.6-5 6.7-2.8-1.1-5-3.5-5-6.7V3.7z" />
    <path d="M5.6 8l1.7 1.7L10.6 6.4" />
  </Icon>
);

export const Check = (p: P) => (
  <Icon {...p}>
    <path d="M3.4 8.4l3 3 6.2-6.6" />
  </Icon>
);

export const Refresh = (p: P) => (
  <Icon {...p}>
    <path d="M13 8a5 5 0 1 1-1.5-3.6" />
    <path d="M13 2.4v3h-3" />
  </Icon>
);

export const Layers = (p: P) => (
  <Icon {...p}>
    <path d="M8 2.2l5.8 3.1L8 8.4 2.2 5.3zM2.2 8.2L8 11.3l5.8-3.1M2.2 11L8 14.1l5.8-3.1" />
  </Icon>
);

export const Pace = (p: P) => (
  <Icon {...p}>
    <path d="M5 13V3M2.6 5.4L5 3l2.4 2.4M11 3v10M8.6 10.6L11 13l2.4-2.4" />
  </Icon>
);

export const Hourglass = (p: P) => (
  <Icon {...p}>
    <path d="M4 2h8M4 14h8M4.8 2c0 3.4 3.2 3.6 3.2 6s-3.2 2.6-3.2 6M11.2 2c0 3.4-3.2 3.6-3.2 6s3.2 2.6 3.2 6" />
  </Icon>
);

export const Sparkle = (p: P) => (
  <Icon {...p}>
    <path d="M8 1.8v12.4M1.8 8h12.4M3.6 3.6l8.8 8.8M12.4 3.6l-8.8 8.8" />
  </Icon>
);

export const Folder = (p: P) => (
  <Icon {...p}>
    <path d="M1.8 4.2a1.4 1.4 0 0 1 1.4-1.4h3l1.6 1.6h5a1.4 1.4 0 0 1 1.4 1.4v6.4a1.4 1.4 0 0 1-1.4 1.4H3.2a1.4 1.4 0 0 1-1.4-1.4z" />
  </Icon>
);
