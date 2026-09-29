/**
 * Monarch tokenizer for Nomic, for Monaco-based editors.
 *
 * Token classes mirror the TextMate grammar's scopes so a theme can treat
 * both the same way. Keywords come from `keywords.json`, the shared source
 * of truth that the Rust parser is tested against.
 */
import keywords from "../keywords.json" with { type: "json" };

/** Structural shape of a Monarch language definition, kept local so this
 *  package does not depend on `monaco-editor` types. Assignable to
 *  `monaco.languages.IMonarchLanguage`. */
export interface MonarchLanguage {
  defaultToken: string;
  tokenPostfix: string;
  ignoreCase: boolean;
  brackets: { open: string; close: string; token: string }[];
  keywords: string[];
  [group: string]: unknown;
  tokenizer: Record<string, MonarchRule[]>;
}

export type MonarchRule =
  | [string | RegExp, string | MonarchAction | (string | MonarchAction)[]]
  | [string | RegExp, string | MonarchAction, string]
  | { include: string };

export interface MonarchAction {
  token?: string;
  next?: string;
  cases?: Record<string, string | MonarchAction>;
  log?: string;
}

const {
  declarations,
  modifiers,
  control,
  builtins,
  relations,
  literals,
  primitives,
} = keywords;

export const nomicMonarch: MonarchLanguage = {
  defaultToken: "",
  tokenPostfix: ".nomic",
  ignoreCase: false,

  brackets: [
    { open: "{", close: "}", token: "delimiter.curly" },
    { open: "(", close: ")", token: "delimiter.parenthesis" },
  ],

  keywords: [...declarations, ...control],
  declarations,
  modifiers,
  control,
  builtins,
  relations,
  literals,
  primitives,

  tokenizer: {
    root: [
      { include: "@whitespace" },

      // declaration heads: `rule "label" on Occurrence` (quoted label), `exception Name on Invariant`
      [
        /\b(rule)(\s+)("(?:[^"\\]|\\.)*")(\s+)(on)(\s+)([A-Za-z_]\w*)/,
        ["keyword.declaration", "", "string", "", "keyword", "", "entity.name.function"],
      ],
      [
        /\b(exception)(\s+)([A-Za-z_]\w*)(\s+)(on)(\s+)([A-Za-z_]\w*)/,
        ["keyword.declaration", "", "entity.name.function", "", "keyword", "", "entity.name.function"],
      ],
      [/\b(ensure)(\s+)("(?:[^"\\]|\\.)*")/, ["keyword.declaration", "", "string"]],
      [/\b(type)(\s+)([A-Za-z_]\w*)/, ["keyword.declaration", "", "type.identifier"]],
      [/\b(model)(\s+)([A-Za-z_]\w*)/, ["keyword.declaration", "", "namespace"]],
      [
        /\b(fact|derive|action|event|invariant|cite)(\s+)([A-Za-z_]\w*)/,
        ["keyword.declaration", "", "entity.name.function"],
      ],
      [/\b(scenario)(\s+)(?=")/, ["keyword.declaration", ""]],
      [/\b(rejected)(\s+)(by)(\s+)("(?:[^"\\]|\\.)*")/, ["keyword", "", "keyword", "", "string"]],

      // citations: relation followed by a locator string
      [/\b(realizes|derives_from|evidences|contradicts|configures|documents)\b(?=\s+")/, "keyword.relation", "@locator"],

      // ranges before plain numbers
      [/-?\d+\.\.-?\d+/, "number.range"],
      [/\d+/, "number"],

      // type annotation: `: Type?`
      [/(:)(\s*)([A-Z]\w*)(\?)?/, ["delimiter", "", "type.identifier", "operator.optional"]],

      // identifiers and keywords
      [
        /[a-z_]\w*/,
        {
          cases: {
            "@declarations": "keyword.declaration",
            "@modifiers": "keyword.modifier",
            "@control": "keyword",
            "@builtins": "support.function",
            "@relations": "keyword.relation",
            "@literals": "constant.language",
            "@default": "variable",
          },
        },
      ],
      [/[A-Z][A-Z0-9_]+\b/, "entity.name.event"],
      [/[A-Z]\w*(?=\()/, "entity.name.function"],
      [
        /[A-Z]\w*/,
        {
          cases: {
            "@primitives": "type.primitive",
            "@default": "variable.constant",
          },
        },
      ],

      // strings
      [/"/, "string", "@string"],

      // operators and delimiters
      [/==|!=|<=|>=|&&|\|\||\?\?|=>/, "operator"],
      [/[<>=!+\-*/%?:|]/, "operator"],
      [/[{}()]/, "@brackets"],
      [/[,;]/, "delimiter"],
    ],

    whitespace: [
      [/[ \t\r\n]+/, ""],
      [/\/\/\/.*$/, "comment.doc"],
      [/\/\/.*$/, "comment"],
    ],

    string: [
      [/[^\\"]+/, "string"],
      [/\\[\\"nt]/, "string.escape"],
      [/"/, "string", "@pop"],
    ],

    locator: [
      [/\s+/, ""],
      [/"/, "string.link", "@locatorBody"],
      ["", "", "@pop"],
    ],

    locatorBody: [
      [/#L\d+(-L\d+)?/, "number.range"],
      [/@[0-9a-f]+/, "constant.pin"],
      [/[^"#@]+/, "string.link"],
      [/[#@]/, "string.link"],
      [/"/, "string.link", "@popall"],
    ],
  },
};
