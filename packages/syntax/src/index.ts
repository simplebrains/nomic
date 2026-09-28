/**
 * @simplebrains/nomic-syntax
 *
 * Editor support for the Nomic language: a TextMate grammar (VS Code,
 * Sublime, GitHub linguist-style highlighters), a Monarch tokenizer
 * (Monaco), a language configuration, and the shared keyword lists.
 */
import keywords from "../keywords.json" with { type: "json" };
import languageConfiguration from "../language-configuration.json" with { type: "json" };
import textMateGrammar from "../grammars/nomic.tmLanguage.json" with { type: "json" };
import markdownInjectionGrammar from "../grammars/nomic.markdown.tmLanguage.json" with { type: "json" };
import { nomicMonarch } from "./monarch.js";

export { nomicMonarch };
export type { MonarchLanguage, MonarchRule, MonarchAction } from "./monarch.js";
export { keywords, languageConfiguration, textMateGrammar, markdownInjectionGrammar };

/**
 * The grammar in the shape Shiki's `langs` option accepts, so markdown
 * renderers built on Shiki (Astro, Rehype, MDX, Docusaurus) can highlight
 * ```nomic and ```nom fences:
 *
 * ```ts
 * import { createHighlighter } from "shiki";
 * import { nomicShikiLanguage } from "@simplebrains/nomic-syntax";
 * const hl = await createHighlighter({ themes: ["github-dark"], langs: [nomicShikiLanguage] });
 * hl.codeToHtml(src, { lang: "nomic", theme: "github-dark" });
 * ```
 */
export const nomicShikiLanguage = {
  ...textMateGrammar,
  name: "nomic",
  displayName: "Nomic",
  aliases: ["nom"],
};

export const languageId = "nomic";
/** `.nom` is the standard extension; `.nomic` is accepted as a legacy alias. */
export const fileExtensions = [".nom", ".nomic"];
export const scopeName = "source.nomic";

/** Every reserved word the parser refuses as a name. */
export const reservedWords: readonly string[] = [
  ...keywords.declarations,
  ...keywords.modifiers,
  ...keywords.control,
  ...keywords.builtins,
  ...keywords.relations,
  ...keywords.literals,
];

/** The subset of Monaco's `languages` API this package needs. */
export interface MonacoLanguagesLike {
  register(language: { id: string; extensions?: string[]; aliases?: string[] }): void;
  setMonarchTokensProvider(languageId: string, provider: unknown): unknown;
  setLanguageConfiguration(languageId: string, configuration: unknown): unknown;
}

/**
 * Register Nomic with a Monaco instance:
 *
 * ```ts
 * import * as monaco from "monaco-editor";
 * import { registerNomic } from "@simplebrains/nomic-syntax";
 * registerNomic(monaco.languages);
 * ```
 */
export function registerNomic(languages: MonacoLanguagesLike): void {
  languages.register({ id: languageId, extensions: fileExtensions, aliases: ["Nomic", "nomic"] });
  languages.setMonarchTokensProvider(languageId, nomicMonarch);
  languages.setLanguageConfiguration(languageId, toMonacoConfiguration(languageConfiguration));
}

/** Convert the VS Code style language-configuration.json into Monaco's shape
 *  (regex strings become RegExp objects). */
export function toMonacoConfiguration(config: typeof languageConfiguration): unknown {
  return {
    comments: config.comments,
    brackets: config.brackets,
    autoClosingPairs: config.autoClosingPairs,
    surroundingPairs: config.surroundingPairs.map(([open, close]) => ({ open, close })),
    wordPattern: new RegExp(config.wordPattern),
    folding: {
      markers: {
        start: new RegExp(config.folding.markers.start),
        end: new RegExp(config.folding.markers.end),
      },
    },
  };
}
