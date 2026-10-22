/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        coterm: {
          surface: "var(--coterm-surface)",
          "surface-raised": "var(--coterm-surface-raised)",
          "surface-panel": "var(--coterm-surface-panel)",
          "surface-sidebar": "var(--coterm-surface-sidebar)",
          "surface-toolbar": "var(--coterm-surface-toolbar)",
          "surface-hover": "var(--coterm-surface-hover)",
          "surface-active": "var(--coterm-surface-active)",
          border: "var(--coterm-border)",
          "border-subtle": "var(--coterm-border-subtle)",
          "border-input": "var(--coterm-border-input)",
          text: "var(--coterm-text-primary)",
          "text-muted": "var(--coterm-text-muted)",
          "text-subtle": "var(--coterm-text-subtle)",
          "text-disabled": "var(--coterm-text-disabled)",
          "text-bright": "var(--coterm-text-bright)",
          accent: "var(--coterm-accent)",
          "accent-hover": "var(--coterm-accent-hover)",
          "accent-active": "var(--coterm-accent-active)",
          "accent-glow": "var(--coterm-accent-glow)",
          danger: "var(--coterm-danger)",
          "danger-surface": "var(--coterm-danger-surface)",
          "danger-border": "var(--coterm-danger-border)",
          focus: "var(--coterm-focus)",
        },
        agent: {
          500: "#8b5cf6", // Purple accent for Agent commands
          900: "#2e1065",
        },
      },
      fontFamily: {
        sans: ["var(--coterm-font-sans)"],
        mono: ["var(--coterm-font-mono)"],
      },
    },
  },
  plugins: [],
};
