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
  log: (s = 16) => svg('<path d="M4 6h16M4 12h16M4 18h10"/>', s),
};
