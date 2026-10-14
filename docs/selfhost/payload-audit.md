Pomiar READ-ONLY realnego runu `c77f6677-f800-483d-bc8a-f04d43b89b10` z `/tmp/antislop-selfhost-00wizn49/state/state.db`.

Nie zmieniono repozytorium ani aktywnej kopii. SQLite otwarto przez `mode=ro` i pobrano dwa transakcyjnie spójne snapshoty: Discovery (1 artifact, 0 reviews, 7 evidence, 2 turns) oraz Planning (2 artifacts, 1 review, 13 evidence, 4 turns). Harness poza repo korzysta z faktycznego `Provider::prepare_execute/prepare_verify` z już zbudowanej biblioteki. Pre-send widoki odtworzono z historycznych `runtime:orchestration` oraz kolejności evidence. Wszystkie CZTERY realne `prompt_hash` i `schema_hash` zgadzają się z SessionLedger. To rozmiary faktycznie wysłanych payloadów, nie estymacja tokenów.

| Faktyczny turn | Prompt UTF-8 bytes | JSON schema bytes | Razem |
|---|---:|---:|---:|
| Discovery executor | 141288 | 15700 | 156988 |
| Discovery verifier | 351026 | 16603 | 367629 |
| Planning executor | 212454 | 16213 | 228667 |
| Planning verifier | 604264 | 17870 | 622134 |

Verifier urósł o 253238 B (72.1%) już pomiędzy Discovery a Planning. Para Discovery miała 524617 B promptów+schematów, para Planning 850801 B.

Konkretne powielenia:

1. Discovery executor: receipt orchestration 47480 B i bezpośredni ten sam kontekst 41071 B = 88551 B, 62.67% promptu. Discovery verifier: bieżący receipt 48641 B + bieżący kontekst 42142 B + historyczny snapshot 47482 B = 138265 B, 39.39%. Planning verifier: analogicznie 51238 + 44531 + 50079 = 145848 B, 24.14%. Każdy snapshot zawiera 240-wierszową mapę ryzyk; sama mapa ma 37342 B w Discovery, 37311 B w Planning. W obrębie Planning aktualna i historyczna mapa ryzyk są identyczne; zmieniają się głównie ledger slots i temporalna kwalifikacja.
2. Ten sam bieżący artifact.content występuje CZTERY razy u verifiera: `Artifact.content`, osobny `Evidence.output`, ten sam receipt ponownie w `Runner results`, oraz `Prior artifacts`. Discovery: 12447 B raw / 12892 B po JSON escaping, czyli 51568 B czterech kopii samej zawartości. Planning: 42436 B raw / 44629 B escaped, czyli 178516 B, 29.54% promptu. Trzy nadmiarowe kopie Planning to 133887 B przed metadanymi.
3. Identyczny aktualny input_snapshot (7614 B) występuje semantycznie 5 razy w verifierze Discovery i 9 razy w Planning: bezpośrednio, w kolejnych snapshot receipts i po obu stronach diffów, mimo że wejścia nie zmieniły się. W Planning osiem nadmiarowych kopii to 60912 B przed dodatkowym escapingiem.
4. Historyczny baseline ma raw output 19615 B / receipt 20447 B i nadal jest przesyłany w Planning, chociaż nie może uzasadniać zaliczenia aktualnych testów. Nie rośnie sam, ale powtarza się w kolejnych turnach.
5. Dependency graph: Discovery 13901 B / 16 nodes, Planning 65277 B / 49 nodes. W Planning mapy source_hashes zajmują 20585 B, treść węzłów 23966 B; doszły m.in. 27 risk_assessment oraz review. To kolejna ścieżka utrzymywania historii po zamknięciu etapu.
6. Pełny katalog 240 ryzyk ma 64873 B i jest wysyłany w każdym verifierze. To 18.48% Discovery i 10.74% Planning. Nie oznaczam całego katalogu jako zbędnego: jego kompletność jest częścią obecnego kontraktu. Redundantne są wielokrotne osadzenia tej samej mapy/treści, nie same wymagane ID.
7. Schemat quality ma stałe 14697 B: 93.61% schematu Discovery executora i 88.52% verifiera. Planning verifier ma 117 check IDs i 10 evidence IDs, lecz cały schema to tylko 17870 B. Rosną przede wszystkim historie i kopie raw treści, nie check schema.

Rozkład Planning verifiera: Artifact 44860 B; osobny artifact-read Evidence 44932 B; Runner results 198615 B; Prior artifacts 57987 B; Dependency graph 65277 B; full risk catalog 64873 B; orchestration current 44531 B + history 50079 B; pinned inputs 7614 B; najnowsze reviewed quality 2591 B. Podane sekcje są rozłączne na poziomie renderera, choć ich treść semantycznie się powtarza.

Delivery: bez zmyślania przyszłych raportów zmieniono TYLKO stage w kopii snapshotu Planning, zachowując historię. Rezultat: executor 327576 B + schema 15092 B; verifier 599996 B + schema 16358 B. To projekcja stage-only, nie ukończony run. Sama zmiana etapu nie usuwa narosłej historii.

Zmierzony Planning artifact dodaje 44861 B do kolekcji wcześniejszych artefaktów, a jego pełny artifact-read receipt następne 44933 B, gdy nadal należy do zestawu aktualnych receipts. To 89794 B na dodatkowy artefakt o tym rozmiarze. Cztery dalsze artefakty tego samego rozmiaru oznaczałyby do 359176 B dalszej powtarzanej historii, jeszcze bez nowych reviews, graph nodes, runner results, input snapshots i refreshes. Po edycji staleness może usunąć część receipts z promptu, lecz kolekcja historycznych artefaktów pozostaje. Dokładnego Delivery nie da się obliczyć przed wygenerowaniem przyszłych wyników. W pojedynczym promptcie wzrost jest liniowy względem zachowanej historii; suma przesyłanych bajtów po kolejnych turnach może rosnąć kwadratowo przez ponowne wysyłanie całej historii.

Codex raportuje rzeczywiste usage: Discovery executor input_tokens=48751, output_tokens=3045; Discovery verifier input_tokens=106809, output_tokens=6070; Planning executor input_tokens=69711, output_tokens=8371. cached_tokens=0 w tych receiptach. Są to raporty CLI; nie przeliczam bajtów na tokeny, cenę ani limit kontekstu i nie zakładam, że obejmują wyłącznie przygotowany prompt.

Ewentualne optymalizacje bez utraty dowodów: jeden rejestr raw dokumentów w obrębie promptu i odwołania ID/hash w pozostałych sekcjach; jeden current orchestration JSON z receipt metadata, historyczne różnice zamiast całej ponownej mapy 240 ryzyk; wspólny snapshot wejść i diff tylko zmienionych ścieżek, gdy before/after mają ten sam hash. Pełne immutable dane pozostałyby w durable store, a model nadal widziałby każdy potrzebny dokument co najmniej raz. Nie wystarczy usunąć raw dowodów i zostawić niedostępne modelowi referencje. W tym zadaniu nie wdrożono żadnej optymalizacji.


## Redukcja promptu kontynuacji tego samego Execution

Po zatrzymaniu próby #5 pobrano read-only jeden rzeczywisty, spójny persisted snapshot z `/tmp/antislop-selfhost-00wizn49/state/state.db`. Oba renderery przygotowały verifier prompt dla tego samego głównego artefaktu Execution oraz tego samego zbioru aktualnych dowodów. Stary renderer uruchomiono przez istniejącą bibliotekę `source/target/debug/libarchitect.rlib`; nowy przez bieżące repozytorium. Nie uruchamiano providera ani nie przebudowywano lub modyfikowano aktywnej kopii.

**To porównanie payloadów kontynuacji tego samego rzeczywistego persisted Execution, a nie rekonstrukcja wcześniejszego wysłanego promptu ani jego pre-send hash.** Snapshot zawiera już historię zakończonych refreshów oraz zatrzymanego verifiera; wcześniejszych pomiarów Discovery/Planning powyżej nie należy z nim utożsamiać.

| Renderer tego samego stanu | Prompt UTF-8 bytes | JSON schema bytes | Razem |
|---|---:|---:|---:|
| Poprzedni renderer | 1242501 | 17534 | 1260035 |
| Rejestr exact raw body, przed fragmentacją | 1130914 | 17534 | 1148448 |
| Rejestr exact raw body i współdzielonych fragmentów JSON | 812968 | 17534 | 830502 |

Końcowa redukcja promptu wynosi **429533 B (34,57%)**. Schemat odpowiedzi i dopuszczalne current evidence IDs pozostały bez zmian. Nie jest to estymacja tokenów ani gwarancja pomieszczenia się w limicie providera.

Profil prostego rejestru wykazał siedem różnych snapshotów orchestration po 45805–52250 B; powtarzała się głównie 240-elementowa mapa przypisań ryzyk. Dodatkowo występowały artefakty 34590–42436 B, latest reviewed quality 19098 B i workspace diff 15635 B. Fragmentacja była skierowana w te powtarzane dokumenty, bez usuwania katalogu ryzyk lub dowodów.

Format `inline-exact-utf8-v2` przechowuje wszystkie potrzebne treści w tym samym promptcie. Sekcje zawierają metadane i odwołania do `documents[sha256]`; dokument zawiera albo dokładny `utf8`, albo uporządkowane `parts`. Rekonstrukcja polega wyłącznie na konkatenacji literalnych fragmentów UTF-8 i dokładnych treści wskazanych dokumentów, bez dodawania separatorów, dekodowania/ponownego kodowania znaków lub normalizowania JSON. Powtarzane fragmenty co najmniej 512 B wybierane są z oryginalnych granic obiektów, tablic i string tokenów dopiero po stwierdzeniu poprawności JSON. Każde odwołanie wewnątrz dokumentu prowadzi do krótszego fragmentu; wszystkie fragmenty są inline. SHA-256 i liczba bajtów opisują odtworzony oryginalny dokument. Zwykły tekst i niepoprawny JSON pozostają dokładnym UTF-8.

Weryfikacja: `tests/prompt_context.rs` odtwarza pełne artefakty, receipts, źródła, provenance graph, spec i katalog 240 ryzyk; jawny tryb pomiaru odtworzył te same dokumenty także z realnego snapshotu Execution. Siedem testów `tests/verifier_orchestration_context.rs` zachowuje rozróżnienie historycznych snapshotów od current evidence schema. Niezależny reviewer uruchomił cztery testy `tests/verifier_prompt_context.rs`, sprawdzając pełną rekonstrukcję, hashe, Unicode/escapes/whitespace, dostępność referencji, malejące długości i brak cykli. Targeted Clippy dla biblioteki i zmienionych testów przeszedł z `-D warnings`.

Pozostały rozmiar obejmuje pełną wymaganą historię i metadane, w tym powtarzane mapy `source_hashes` w grafie. Nie zastosowano dalszych zmian semantyki dowodów ani dodatkowej kompresji metadanych.


## Supplemental continuation: lossless metadata/string factoring

Measurements below use the SAME frozen persisted `attempt-6-blocked-run.json` (8 artifacts, 6 reviews). They are rendered verifier requests from that blocked snapshot, **not claims of the exact earlier pre-send prompt**. Its source state is unchanged between renderings; the schema remains 18342 bytes.

| Presentation | Prompt bytes | Schema bytes | Total bytes |
|---|---:|---:|---:|
| v2 exact UTF-8 parts, before these changes | 1212601 | 18342 | 1230943 |
| v3 typed metadata refs + slim quality runner metadata | 1053708 | 18342 | 1072050 |
| v4 canonical JSON string token refs | 985784 | 18342 | 1004126 |

v3 restores source-hash maps/arrays through `json_value_ref`: decode the referenced original JSON once, without interpreting reference-shaped values inside the result. Literal singleton reference-shaped metadata is itself protected this way. Quality source bindings remain a complete inline typed array; runner instruction summaries carry id/source_kind/source_hash/output_hash while full source strings, literal argv/programs and outputs remain in the context registry.

v4 adds `json_string_ref` only for original JSON string tokens whose deterministic serde_json encoding of the decoded string exactly reproduces the original token bytes. All referenced children are strictly shorter; parent hashes describe fully reconstructed original bytes. It also factors the literal `runner:` prefix from an exact valid JSON argv suffix. Noncanonical escape spellings, numbers, whitespace and object order are never normalized. This saves 226817 prompt bytes (18.70%) from v2 on this frozen snapshot. Full risks, historical artifacts, current evidence eligibility and temporal narrative remain present. The approximately 1 MB remaining payload is a limitation; these measurements do not prove the cause of prior CLI exit 1 or guarantee successful continuation.


## v5: proof closure and local document aliases

On the SAME frozen `attempt-6-blocked-run.json` (file SHA-256 `43566aee614321b0dc6b37f8949db28d08135bb0f6149c13f0009872eaea2d3e`), the final v5 verifier rendering is **902839 prompt bytes + 17952 schema bytes = 920791 bytes**. Prompt SHA-256: `d45725d9b1481ae51318b5d97b618b0ee880bdff14b93adbd63c10386e696327`; schema SHA-256: `34900680478029709556507a79bc96e510bf352800083a79a59bcb7748974d54`. This is a persisted-snapshot comparison, not the exact earlier pre-send payload.

The projection retains 6/8 artifacts, 133/241 provenance nodes, 19/29 currently eligible receipts and 11 additional historical proof receipts (30/54 receipts total). All **124 expected checks** and gate definitions remain unchanged. Full actual bodies of two authenticating reviews and six persisted gate assessments are included. Required historical Execution 3 remains because the actual expected checklist contains its `REVALIDATE` check; its original Discovery/Planning ancestors therefore remain too. Only unrelated rejected Execution 4/5 artifact narratives are omitted. Their needed actual test receipts remain when cited by selected quality/artifacts. Failed receipts against the pinned artifact are always retained, even when uncited.

Historical proof receipts expose exact referenced bodies but are excluded from the current response evidence enum. The selected current enum is a stricter subset of the original eligible IDs. Durable history is not changed. Local document IDs (`d1`, ...) replace only manufactured reference values; `sha256` still binds reconstructed original bytes. Opaque strings and decoded literal data are never alias-rewritten, and all three singleton reserved wrapper shapes are protected. Offline reconstruction checked all 314 document hashes/byte lengths and compared selected actual artifacts, current/historical receipts, gates and review DTOs with the frozen export.

An initial proof-selection estimate without the actual authenticating review/gate bodies was 871929 prompt bytes; including those bodies before aliases was 960078. The final measured projection includes them. These figures do not establish why a native CLI failed or guarantee later model acceptance. The live run may continue beyond this frozen audit; this change does not intervene in an active native session.


## Validation continuation: native scalar limit and compact v6 registry

The native Codex `turn/start` diagnostic confirmed an input limit of **1048576 Unicode scalar characters**; UTF-8 byte size and model token counts are different quantities. The observed rejected native input contained 1290156 characters. Measurements below compare renderer output on saved snapshots; they do not relabel those snapshots as actual pre-send payloads. Schema bytes are reported separately.

| Same frozen `validation-blocked-run.json` experiment | Prompt scalar characters | Prompt UTF-8 bytes | Schema bytes |
|---|---:|---:|---:|
| Before these lossless changes | 1284615 | 1286153 | 21611 |
| Whole generated DTO metadata + repeated UTF-8 blocks | 1178018 | 1179544 | 21611 |
| Explicit current proof review-row closure | 1109184 | 1110710 | 21299 |
| Compact v6 parts + frequent exact tokens | 940672 | 942198 | 21299 |

Final frozen-snapshot prompt SHA-256: `127e2515b053e66a69feb4b5b866c52a450b4cbd8388026d7113d9405e85d61d`; schema SHA-256: `899b2397859141e7a55eb7b7462e524063f6f760ccfa7c8a5da9ec6f3b267d03`. This leaves 107904 characters below the confirmed native limit. The desired 900000-character margin was not reached; the root coordinator accepted the measured headroom and requires per-invocation preflight for subsequent actual payloads. No forecast of as-yet-unwritten Integration/Delivery artifacts is claimed.

A later single read-only database snapshot (state SHA-256 `39487cbc410af10707b3017ec62886d1178bda3fffcd0d9723f7acd765bd47d5`) still showed Blocked Validation 8, 10 artifacts, 9 reviews and `reconciliation_required=true`. Rendering that snapshot yielded **944271 scalar characters / 945797 UTF-8 bytes**, with a 21377-byte schema; prompt SHA-256 `f423aa576f42f366b1b6f240739a3f8245ecacd6a38fb31187332a3a4c902d04`, schema SHA-256 `8399c36ba8abf14fc5a17b835115dc1df664b4368ec18cef3144a11b399bf316`. This is explicitly a current-snapshot rendering before reconciliation, not proof of a later actual sent payload. The 3599-character increase demonstrates why the native preflight remains authoritative.

`inline-exact-utf8-v6` keeps original `sha256`/`utf8_bytes` for every reconstructed body. Document `parts` now contains literal strings, positive integers inserting raw `documents[dN]`, or negative integers inserting the checked canonical JSON-string encoding of `documents[dN]`; zero is invalid. This grammar applies only to manufactured document parts, never to arrays or reference-shaped values inside original decoded data. Generated DTO metadata uses typed JSON views; original file, artifact and runner-output strings remain exactly reconstructable. Repeated long UTF-8 blocks and frequent short exact tokens share hash-bound bodies only when the estimated storage saving pays their reference/document overhead. Unreachable presentation seeds are removed; required raw data is not truncated.

Current proof review check IDs now explicitly identifies risk/gate/revalidation rows used by the current proof. Full original authenticating Review DTOs remain inline. Unrelated historical control labels no longer cause unrelated artifact narratives to become current proof roots. Actual current context/model roots follow the runtime's latest-stage prerequisite selection; all selected dependencies and explicit claims remain. The latest primary checker, original negative baseline and explicitly cited supplemental checks remain. Uncited obsolete probe V1 is omitted; V2 and the actual benchmark remain. Some earlier Discovery/Planning artifact-read bodies remain because current risk assessments explicitly cite them.

Production `src/providers/prompts.rs` is frozen at SHA-256 `09628643685264e586af58c455952e66c5d32607e7749a55cb83765010074d43`. Own verification passed 59 tests across lib/core/quality/prompt/temporal suites, followed by strict targeted Clippy. This records lossless presentation, source/proof closure and literal schema checks; it does not claim semantic acceptance of a future native verifier output or final completion of the selfhost run.


### Compact registry acceptance fixes before coordinator v7.1

The first retained v7 source remains a historical build. Full-suite verification exposed a stale recorded gate with current source hashes but missing authenticating Review: such a gate correctly requires a fresh expected check, and must not become an authenticated proof root. The prompt projection now calls the existing backend `assessment_is_current` predicate (its visibility alone changed to `pub(crate)`). All recorded GateAssessment DTOs remain inline and fresh expected GATE checks remain unchanged.

On the same frozen Validation snapshot, this changes only the decoded `Current proof review check IDs` section: the stale `GATE:context_ready:design` row is no longer labeled as a current proof binding. Every other decoded section is unchanged. The new rendering is **940642 scalar characters / 942168 UTF-8 bytes**, prompt SHA-256 `0a03e0c6334efdd334c95222f0108f53d20bdb19825cca48342d089853137a78`; schema remains 21299 bytes with SHA-256 `899b2397859141e7a55eb7b7462e524063f6f760ccfa7c8a5da9ec6f3b267d03`. This remains a persisted-snapshot rendering, not a later native pre-send claim.

A sampled six-stage fixture also showed repeated `expected_checks` computation inside the gate-filter loop. Computing that same immutable set once removes redundant validation work without changing the emitted instructions. The isolated six-stage core test passed before and after, with measured wall time 15.99s then 13.06s, and user CPU 21.92s then 16.90s. These single observations are not a benchmark guarantee; concurrent full-suite runs had exposed the existing five-second per-stage test deadline. No deadline or FakeProvider bypass changed. The targeted default-parallel suite passed 52 tests; strict targeted Clippy and formatting checks passed.

Frozen production prompt source for this correction: SHA-256 `b011f56e863fe81e9e231e465a5fff29a91bc82a6fb523da4d48fbe940991616`. Quality source SHA-256 `f534298075bd06bec76e4abdc151481103a6b4b1b2f7be11d78d7605741dd03e`; the worker's only quality-module change was visibility of the existing predicate. Independent v6 preservation review passed 12 tests before these narrow corrections, including both frozen runs; independent recheck of the correction and full immutable coordinator suites remain separately attributable results.


## Integration 9: compact v7 tuples and profitable exact short blocks

The actual native preflight rejected Integration 9 before invoking the CLI: **1084551 scalar characters**, above the confirmed 1048576 limit. The failed invocation had no observed native session ID or partial effects. Offline rendering of saved `integration9-blocked-run.json` includes that subsequent ledger state and therefore measures **1086556** characters before this codec change; it is not relabeled as the earlier rejected pre-send payload. The frozen export SHA-256 is `292ddb7ed7fcb5ede51a41fda3867749ead4f818b9df5acd6c7bc355bcbff939`.

| Same frozen Integration 9 state | Prompt scalar characters | Prompt UTF-8 bytes | Schema bytes |
|---|---:|---:|---:|
| Prior compact v6 registry | 1086556 | 1088082 | 22692 |
| v7 document tuples + profitable 64–256-byte exact blocks | 969354 | 970878 | 22692 |
| Additional profitable 16–96-byte exact blocks | 930022 | 931546 | 22692 |

The final prompt SHA-256 is `1d73cadeffde153809360f89c774a588c1e813af651ece07c664beb7ff6ec92d`; schema SHA-256 is `dd0de445e4e4968865e3f324a44f59208cda58eee7540cb0adb9184e994e8980`. Reduction is 156534 scalar characters; hard-limit headroom is **118554**. The desired approximately 850000-character target was not reached. The coordinator accepted the measured headroom for the next actual Integration invocation; no guarantee about an unwritten Delivery artifact or later prompt is implied.

A single subsequent SQLite read-only transaction showed Blocked Integration 9, 11 artifacts, 10 reviews and `reconciliation_required=true`. Raw stored-state SHA-256 was `961657391c6a55464a2aab934c24947a21bf447292e3d3653772335871b050f7`. Rendering this current snapshot produced the same 930022 characters, prompt hash and schema hash above. This is a snapshot audit before reconciliation/resume, not a claim that the later actual native request has already been sent.

Format `inline-exact-utf8-v7` stores each document as exactly `[sha256, utf8_bytes, body]`. String body means complete literal UTF-8; array body is the existing ordered parts grammar: literal string, positive integer raw child, negative integer canonically JSON-string-encoded child. Zero remains invalid; child bytes are strictly shorter; all referenced bodies remain inline. The grammar applies only at the manufactured document table, never to original user arrays or typed values. It preserves the exact original bytes, whitespace, escape spellings, numeric spellings and hashes. Short blocks use content-defined UTF-8 boundaries and are shared only when repetition pays conservative complete-tuple/hash/reference overhead. No proof data is omitted to meet a size target.

The accompanying retry fix preserves the original available proof closure but requires a hash-authenticated actual Review DTO before treating a recorded risk binding as a proof root. Matching artifact/authorship/revision/checklist/check/evidence/rationale fields are checked. Actual refresh DTOs in `provenance_reviews` remain available as historical proof even when absent from `reviews`; their inclusion is not a declaration that the backend considers an earlier risk assessment current. A descriptive `review:<hash>` graph label is never substituted for a missing DTO. Unchanged fresh expected risk checks and validators remain authoritative. The original eight independent verification-retry tests passed without weakening their assertions.

Frozen prompt source SHA-256: `555cb96af16938a03cbc87857f442a7e2cf4fdc6af1e50db35b7734b17dfb8c5`. Own core/prompt/temporal/retry suite passed 54 tests, followed by strict targeted Clippy. Independent review passed 25 tests, including saved Integration 9, saved Validation 8, saved Execution 6, raw-byte/hash reconstruction, typed/literal-wrapper and document-shaped user arrays, proof ancestry, review bindings, complete expected checks/gates and current-versus-historical evidence eligibility. Independent strict Clippy passed; a fresh six-stage core cycle passed with the existing five-second per-stage deadlines. These results establish tested presentation and recovery behavior, not semantic acceptance of a future model response or completion of the live selfhost run.
