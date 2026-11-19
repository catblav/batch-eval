/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{vue,js}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // 冷中性工具色板，亮/暗共用同一批语义类名
        page: "rgb(var(--page) / <alpha-value>)",
        panel: "rgb(var(--panel) / <alpha-value>)",
        sidebar: "rgb(var(--sidebar) / <alpha-value>)",
        wash: "rgb(var(--wash) / <alpha-value>)",
        line: "rgb(var(--line) / <alpha-value>)",
        // 卡片描边，比 line 亮一档；暗色下阴影几乎不可见，靠它勾出卡片边缘
        cardline: "rgb(var(--card-line) / <alpha-value>)",
        main: "rgb(var(--text-main) / <alpha-value>)",
        sub: "rgb(var(--text-sub) / <alpha-value>)",
        weak: "rgb(var(--text-weak) / <alpha-value>)",
        btn: "rgb(var(--btn) / <alpha-value>)",
        btnfg: "rgb(var(--btn-fg) / <alpha-value>)",
        accent: "rgb(var(--accent) / <alpha-value>)",
        pass: "rgb(var(--pass) / <alpha-value>)",
        fail: "rgb(var(--fail) / <alpha-value>)",
        warn: "rgb(var(--warn) / <alpha-value>)",
      },
      fontFamily: {
        sans: [
          "Segoe UI Variable Text",
          "Segoe UI",
          "Microsoft YaHei UI",
          "Microsoft YaHei",
          "PingFang SC",
          "system-ui",
          "sans-serif",
        ],
        mono: ["Cascadia Code", "Consolas", "JetBrains Mono", "monospace"],
      },
      boxShadow: {
        card: "var(--shadow-card)",
        pop: "var(--shadow-pop)",
      },
    },
  },
  plugins: [],
};
