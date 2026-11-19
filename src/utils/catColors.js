// 错误分类的区分色：同一分类名永远同色，不同分类尽量不同色。
// 用「首次出现顺序分配」而不是哈希：同一分类名跨组件/跨刷新稳定，新分类
// 取色板里下一个未用色，所以列表里不同分类不会撞色（色板用尽后才轮换）。
// 分类分布卡、明细表格的分类标签、详情抽屉共用这一套，保证全工具一致。
const COLOR_COUNT = 12;
const CAT_COLORS = Array.from({ length: COLOR_COUNT }, (_, i) => `rgb(var(--cat-${i}))`);
const CAT_SOFT = Array.from({ length: COLOR_COUNT }, (_, i) => `rgb(var(--cat-${i}) / 0.10)`);
const CAT_LINE = Array.from({ length: COLOR_COUNT }, (_, i) => `rgb(var(--cat-${i}) / 0.30)`);

const nameIndex = new Map();
let nextIndex = 0;

function indexOf(name) {
  if (!nameIndex.has(name)) {
    nameIndex.set(name, nextIndex % COLOR_COUNT);
    nextIndex++;
  }
  return nameIndex.get(name);
}

export const catColor = (name) => CAT_COLORS[indexOf(name)];
export const catSoft = (name) => CAT_SOFT[indexOf(name)];
export const catLine = (name) => CAT_LINE[indexOf(name)];
