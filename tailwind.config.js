/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{vue,js}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // 全部走 CSS 变量，亮/暗两套主题共用同一批类名
        ink: {
          950: "rgb(var(--ink-950) / <alpha-value>)",
          900: "rgb(var(--ink-900) / <alpha-value>)",
          850: "rgb(var(--ink-850) / <alpha-value>)",
          800: "rgb(var(--ink-800) / <alpha-value>)",
          700: "rgb(var(--ink-700) / <alpha-value>)",
          600: "rgb(var(--ink-600) / <alpha-value>)",
          500: "rgb(var(--ink-500) / <alpha-value>)",
          400: "rgb(var(--ink-400) / <alpha-value>)",
          300: "rgb(var(--ink-300) / <alpha-value>)",
          200: "rgb(var(--ink-200) / <alpha-value>)",
        },
        pass: "rgb(var(--pass) / <alpha-value>)",
        fail: "rgb(var(--fail) / <alpha-value>)",
        warn: "rgb(var(--warn) / <alpha-value>)",
        accent: "rgb(var(--accent) / <alpha-value>)",
      },
      fontFamily: {
        sans: ["Segoe UI", "Microsoft YaHei", "PingFang SC", "system-ui", "sans-serif"],
        mono: ["Cascadia Code", "Consolas", "JetBrains Mono", "monospace"],
      },
    },
  },
  plugins: [],
};
