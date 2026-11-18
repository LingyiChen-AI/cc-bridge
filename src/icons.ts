// Lucide 风格线性图标（24 网格，stroke 2）
const svg = (body: string, size = 18) =>
  `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;

export const icon = {
  logo: (s = 22) =>
    svg('<circle cx="5.5" cy="5.5" r="2.5"/><path d="M8 5.5h6.5a3.25 3.25 0 0 1 0 6.5h-5a3.25 3.25 0 0 0 0 6.5H15"/><circle cx="18.5" cy="18.5" r="2.2" fill="currentColor"/>', s),
  server: (s = 18) => svg('<rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/>', s),
  key: (s = 18) => svg('<circle cx="7.5" cy="15.5" r="4.5"/><path d="m10.7 12.3 9.3-9.3M17 6l3 3M14 9l2 2"/>', s),
  terminal: (s = 18) => svg('<path d="m4 17 6-5-6-5"/><path d="M12 19h8"/>', s),
  refresh: (s = 16) => svg('<path d="M21 12a9 9 0 1 1-2.6-6.4L21 8"/><path d="M21 3v5h-5"/>', s),
  copy: (s = 16) => svg('<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/>', s),
  chevron: (s = 16) => svg('<path d="m6 9 6 6 6-6"/>', s),
  check: (s = 16) => svg('<path d="M20 6 9 17l-5-5"/>', s),
  alert: (s = 18) => svg('<path d="M12 9v4M12 17h.01"/><path d="M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z"/>', s),
  arrowRight: (s = 16) => svg('<path d="M5 12h14M13 6l6 6-6 6"/>', s),
  arrowLeft: (s = 16) => svg('<path d="M19 12H5M11 18l-6-6 6-6"/>', s),
  settings: (s = 16) =>
    svg('<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z"/>', s),
  close: (s = 16) => svg('<path d="M18 6 6 18M6 6l12 12"/>', s),
  power: (s = 16) => svg('<path d="M12 2v10"/><path d="M18.4 6.6a9 9 0 1 1-12.77.04"/>', s),
  home: (s = 16) => svg('<path d="M3 10.5 12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z"/>', s),
  layers: (s = 16) => svg('<path d="m12 2 9 5-9 5-9-5 9-5Z"/><path d="m3 12 9 5 9-5"/><path d="m3 17 9 5 9-5"/>', s),
  activity: (s = 16) => svg('<path d="M22 12h-4l-3 9L9 3l-3 9H2"/>', s),
  search: (s = 16) => svg('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>', s),
  download: (s = 16) => svg('<path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/>', s),
  globe: (s = 16) => svg('<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>', s),
  boot: (s = 16) => svg('<path d="M12 2v4M12 18v4M4.9 4.9l2.8 2.8M16.3 16.3l2.8 2.8M2 12h4M18 12h4"/>', s),
  lang: (s = 16) => svg('<path d="M4 5h8M8 3v2M5 9c1.5 3 4 5 7 6M11 5c-1 4-3 7-7 9"/><path d="m13 21 4-9 4 9M14.5 18h5"/>', s),
  folder: (s = 16) => svg('<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>', s),
  zap: (s = 16) => svg('<path d="M13 2 3 14h9l-1 8 10-12h-9z"/>', s),
  right: (s = 16) => svg('<path d="m9 18 6-6-6-6"/>', s),
  log: (s = 16) => svg('<path d="M4 6h16M4 12h16M4 18h10"/>', s),
};
