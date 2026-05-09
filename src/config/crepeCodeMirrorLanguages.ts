/**
 * Crepe 代码块语言下拉：在 @codemirror/language-data 基础上追加 Mermaid。
 * 无官方 Mermaid Lezer 包时，用 JavaScript 词法高亮提升可读性（与渲染预览独立）。
 */
import { javascript } from "@codemirror/lang-javascript";
import { LanguageDescription } from "@codemirror/language";
import { languages as defaultCodeMirrorLanguages } from "@codemirror/language-data";

const mermaidDescription = LanguageDescription.of({
  name: "Mermaid",
  alias: ["mermaid"],
  load() {
    return Promise.resolve(javascript({ jsx: false, typescript: false }));
  },
});

export const crepeCodeMirrorLanguages: LanguageDescription[] = [
  mermaidDescription,
  ...defaultCodeMirrorLanguages,
];
