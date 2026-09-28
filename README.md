# nomic

Nomic Core 0.1 reference machine: a deterministic executable specification
language, in Rust.

A Nomic model is an executable domain model rather than a program. It declares
what exists (types and facts), what is true by consequence (derived values),
what can happen (actions and events), what follows when something happens
(rules with explicit effects), what must always hold (invariants), where the
real system is known to break the rules (exceptions), and concrete executable
claims about all of it (scenarios). The machine runs the model
deterministically and explains every transition.

```
$ nomic run examples/connect_four.nom
PASS "vertical win is emitted exactly once and ends the game"
PASS "horizontal win"
...
6/6 scenario(s) passed

$ nomic explore examples/tic_tac_toe.nom
explored 5478 state(s), 16167 transition(s), 82437 rejected attempt(s), 958 terminal, max depth 9
exhaustive: yes
```

## Repository layout

```
crates/nomic        the reference machine: library and `nomic` CLI (Rust)
packages/syntax     editor support: TextMate grammar, Monaco Monarch tokenizer,
                    language configuration, shared keyword list (TypeScript)
packages/vscode     VS Code extension wrapping the grammar
examples/           eight example models
fixtures/           generated conformance fixtures, one per model
```

Rust crates form a Cargo workspace; TypeScript packages form a pnpm
workspace. A language server (`crates/nomic-lsp`) over the Rust checker is the
next planned crate.

## Build and run

```
cargo build --release          # the machine
./target/release/nomic help
cargo test                     # models, fixtures, position counts, citation drift, keyword sync

pnpm install && pnpm build     # editor packages
pnpm test
pnpm --filter nomic-vscode dev:link   # symlink the extension into VS Code, then reload
```

Commands:

| command | what it does |
| --- | --- |
| `check <model>` | parse and statically check |
| `ir <model>` | print the canonical JSON IR |
| `run <model> [-v] [--json]` | run every scenario as a conformance test |
| `actions <model>` | list legal actions from the initial state |
| `play <model> <occurrence>...` | apply occurrences from the initial state, printing the rule/effect/event trace |
| `eval <model> <expr>` | evaluate an expression against the initial state |
| `explore <model> [--depth N] [--max-states N]` | bounded exhaustive exploration; every transition verifies the invariants, and a violation comes back with the path that reaches it |
| `fixture <model> [--out f.json]` | generate a language-neutral conformance fixture from the scenarios |
| `conform <model> <fixture.json>` | check the model against a stored fixture |
| `report <model>` | knowledge status (required/observed/expected/assumed), exceptions, citations, description coverage |
| `cite <model> [--root DIR] [--pin] [--index]` | resolve every citation against the repository, report `current`, `stale`, `unverified`, or `unresolved`; `--pin` writes the current content hash into each locator; `--index` groups citations by cited file |

Models use the `.nom` extension.

## The language in one page

```nomic
/// Doc comments are the description channel and land in the IR.
model ConnectFour

type Player = Red | Yellow          // enum; declaration order is the total order
type Column = 0..6                  // inclusive integer range; finite, so quantifiable

fact Cell(col: Column, row: 0..5): Player   // functional: key tuple -> value, or none
fact Turn: Player                            // zero keys: a variable
fact Drawn                                   // no value: a flag (present or absent)

derive Height(c: Column): Int = count(r: 0..5 => Cell(c, r) != none)   // pure
derive Other(p: Player): Player = match p { Red => Yellow, Yellow => Red }

action Drop(player: Player, col: Column)     // an attempted occurrence

rule TurnOrder on Drop(p, c) { require Turn == p "not your turn" }
rule Place on Drop(p, c) {
  require Height(c) < 6 "column is full"
  assert Cell(c, Height(c)) = p           // explicit effects, evaluated against the pre-state
  assert Turn = Other(p)
}

event Win(p: Player) when FourInARow(p)      // edge-triggered: emitted once, when it becomes true
rule RecordWinner on Win(p) { assert Winner = p }   // reaction, same transition

invariant NoFloatingDiscs:
  all(c: Column, r: 0..5 where r > 0 && Cell(c, r) != none => Cell(c, r - 1) != none)

init { assert Turn = Red }

scenario "vertical win" {
  Drop(Red, 0); Drop(Yellow, 1); Drop(Red, 0); Drop(Yellow, 1); Drop(Red, 0); Drop(Yellow, 1)
  Drop(Red, 0) emits Win(Red)
  Drop(Yellow, 2) rejected by GameOver
  expect Winner == Red
}
```

Other constructs, each introduced by one of the example models:

- `??` none-coalescing, `T?` optional derive results, `first(...)` deterministic selection, `sum`, `exists`, `all`, `count`.
- `for (x: T where cond) { effects }` bounded effect comprehension (checkers, chess initial positions).
- `ensure Name: expr` postcondition over an action's own effects that **rejects** the action if it fails (chess: a move may not leave one's own king in check). Distinct from `invariant`, whose violation is a model error.
- `legal(Action(args))` built-in predicate: would the action be accepted now? Lets "no legal move" be a derived value (checkmate, stalemate).
- `observed | expected | assumed` status prefixes on declarations, and `exception Name on Invariant when cond` naming a known departure from a normative invariant. The verifier reports when an exception is exercised instead of failing (inventory audits, the ship's red-alert power overdraw).
- `given nothing` / `given Fact(k) = v` scenario steps to start from a constructed state.
- `rule R on A(p) when cond { ... }`: an applicability guard. False means the rule does not match, unlike `require`, which means the occurrence is forbidden. Needed for anything that dispatches on state, such as a stage machine.
- **Citations.** Any declaration (or the model) may carry trailing `realizes | derives_from | evidences | contradicts | configures | documents "path#Lstart-Lend[@pin]" ["note"]` clauses grounding it in a file. `nomic cite` resolves them and detects drift by content hash; `contradicts` records a known gap between intent and code. `examples/nomic.nom` describes this machine's own pipeline with 22 citations into `src/`.

## Editors and markdown

- **VS Code / Cursor:** `pnpm --filter nomic-vscode dev:link`, then reload. `.nom` files get the Nomic grammar, and fenced blocks tagged ```` ```nomic ```` or ```` ```nom ```` highlight inside markdown files and in the markdown preview, via an injection grammar the extension contributes.
- **Monaco:** `registerNomic(monaco.languages)` from `@simplebrains/nomic-syntax`.
- **Your own markdown renderer (Astro, MDX, rehype, anything on Shiki):** pass `nomicShikiLanguage` from `@simplebrains/nomic-syntax` in Shiki's `langs`; fences tagged `nomic` or `nom` then highlight.
- **GitHub:** cannot be taught per repository. Fences render as plain text until Nomic is added to GitHub Linguist, which requires a published grammar and real-world usage. Tag fences `nomic` anyway so they light up everywhere else.

## The transition pipeline

`Machine::apply(state, occurrence)` runs the reference pipeline from the core
semantics note:

1. **typecheck** the occurrence; 2. **match** rules by pattern; 3. **check**
requirements and denials; 4. **resolve** (any denial rejects; no matching rule
rejects); 5. **transition**: union the effects, fail on conflicts (two rules
disagreeing about one fact), apply; 6. **derive** on demand; 7. **detect**
condition-backed events by edge; 8. **react**: rules on events add effects in
further rounds until no new events; 9. **verify** invariants, consulting
exceptions; 10. **commit** the next state, the event set, and the trace.

Same model, state, and occurrence always give the same result: rules see the
pre-state, effects are a set with an explicit conflict policy, and every
enumeration is in canonical type order.

## Examples

| model | what it exercises | scenarios |
| --- | --- | --- |
| `tic_tac_toe` | smallest complete game; explored exhaustively (5478 positions, 958 terminal) | 3 |
| `connect_four` | the Core 0.1 conformance model; ply counts match OEIS A212693 | 6 |
| `checkers` | multi-jump via reaction rounds, mandatory capture, crowning, no-move loss | 8 |
| `chess` | sliding pieces, castling, en passant, promotion, `ensure` for check, `legal()` for mate | 9 |
| `inventory` | descriptive modeling: reservations, audits, named exceptions to a strong invariant | 6 |
| `dungeon` | RPG with dice as action parameters, initiative, death reactions, item conservation | 7 |
| `ships_computer` | ordered enums as rank, power budget with observed overdraw, two-officer self-destruct | 11 |
| `nomic` | the machine's own pipeline as a stage machine, every rule citing the Rust that realizes it; one honest `contradicts` | 8 |

`fixtures/*.json` are the generated conformance fixtures for each; `cargo test`
checks every model, every scenario, every fixture, and the tic-tac-toe and
Connect Four position counts.

## Status

This is Core 0.1: the semantic kernel and a compact brace syntax that follows
it. Not here yet: the markdown authoring microformat, solver-backed proof
(rung five of the verification ladder), processes and external contracts from
the v6 notes, and non-finite domains.
