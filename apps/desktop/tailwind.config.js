/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        agent: {
          500: "#8b5cf6", // Purple accent for Agent commands
          900: "#2e1065",
        },
      },
    },
  },
  plugins: [],
};
