# Audyt kontynuacji istniejącego runu

Stan oceniony read-only 2026-10-13: run `c77f6677-f800-483d-bc8a-f04d43b89b10`, workspace `/tmp/antislop-selfhost-00wizn49/source`, SQLite otwarte przez `mode=ro`, odczyty state/events w transakcyjnie spójnych snapshotach. Nie uruchamiano native CLI providera, nie modyfikowano bazy, aktywnej kopii ani jej plików. Harnessy pomiarowe i ich wejściowe snapshoty zapisano poza aktywną kopią, w `/tmp`.

## Tożsamość i reconciliation

Kontynuacja zachowuje ten sam run, spec i revision 1. Pinned input snapshot pozostał taki sam jak przed reconciliation; rzeczywiste cztery pinned files odpowiadały temu snapshotowi. Pięć wcześniejszych artefaktów i cztery wcześniejsze reviews zachowano bez zmian. W porównaniu z `source-hashes.json` jedynym zmienionym plikiem źródłowym był `src/main.rs`.

`recovery-journal.json`, `reconcile-response.json` i `repair-response.json` dokumentują kolejność: obserwacja już zastosowanej zmiany, jawne reconciliation, wyczyszczenie pending edit intents, jawne ApproveRepair, nowa próba Execution 4. Reconciliation nie wróciło do Discovery ani nie podniosło revision; zachowało aktualny kod i historię. Nowa próba otrzymała świeży turn i native session, zamiast ponownie wysłać stary turn o niepewnym wyniku. Poprzedni nieukończony verifier pozostał w ledger jako Failed.

Odczyt events potwierdził `effects_reconciled` → `repair_approved` → `attempt_started` → `turn_reserved` → provider events. Przy dalszym Running ponowne `reconciliation_required=true` jest flagą nowej aktywnej próby, a nie cofnięciem wykonanego reconciliation.

## Faktycznie wysłane payloady nowej próby

Odtworzono pre-send widoki z trwałych danych, bez wywoływania providera. Dla executora użyto `reconciled-run.json`, rzeczywistych approvals/evidence do pre-turn orchestration receipt i nowego attempt 4. Dla verifiera odjęto wyłącznie jego świeżą reserved session/turn od odczytanego snapshotu. W obu przypadkach odtworzony orchestration context był identyczny z odpowiednim durable pre-turn snapshotem; przygotowane prompt/schema hashes były identyczne z SessionLedger.

| Rzeczywisty turn próby 4 | Prompt UTF-8 bytes | JSON schema bytes | Razem |
|---|---:|---:|---:|
| Execution executor | 314619 | 15092 | 329711 |
| Execution verifier | 842906 | 17718 | 860624 |

Executor turn `68c5d08a-cac2-4467-9b17-fe6443681f4e`:

- prompt SHA-256: `3799d6d4a26b9ea1bcb683a8477203949ba48b19bfe2cf5a05a0b7437dc6485b`
- schema SHA-256: `9fd317ae9c6f6d3eeee144821fda74cc5b55e49d84e6cea03e45176297b6f1c4`
- observed native session: `01a134f8-7b6c-7f41-8064-71a1180d5f73`

Verifier turn `f7c2787b-e425-406e-befb-57a7775400a8`:

- prompt SHA-256: `e24ed41e25696e46555cca899b13f1e3a21fa2098e1d4fa976725018695e6b2d`
- schema SHA-256: `a071b02bc7ea2f3b46377e9180106be45e2a70e5aa54887a16ce9728f05d2015`
- observed native session: `01a134fb-d896-78b3-9556-c11769d149ec`

To rzeczywiste nowe payloady potwierdzone zgodnością hashy, a nie wcześniejsze porównanie rendererów ze stanu po zatrzymaniu. Rozmiar verifiera różni się od projekcji 812968 B w `payload-audit.md`, ponieważ nowa próba dopisała artefakt Execution 4 i nowe dowody/provenance.

## Native independence i aktualne bramki

Powyższe native UUID są różne także po kanonizacji UUID; nie jest to jedynie porównanie dwóch logical session IDs. W odczytanym ledger wszystkie obserwowane pary provider/native UUID były unikalne. Nowy executor zakończył turn, a jego raw output i output reference były trwałe. W ostatnim ocenionym snapshotcie nowy verifier miał stan Sent, `provider_completed=false`, bez committed output; run nadal był Running w Execution 4.

Sześć design gates miało Pass: context_ready, solution_selected, plan_ready, architecture_verified, input_domain_verified i dependencies_verified. Były obecne świeże wcześniejsze context/planning reviews; nie stwierdzono jeszcze nowego Execution review ani końcowego przejścia delivery gates. **Ten audyt nie potwierdza Completed ani końcowej akceptacji self-hostingu.** Wymaga to późniejszego odczytu zakończonego runu, aktualnych reviews/receipts/gates i końcowych testów.

## Pochodzenie binariów i granice dowodu

Nie był to nieprzerwany przebieg tego samego binarium koordynatora. Journal hash `24fedc189b54670123dfe727390280d9c879862e4c380e655f10bc52ca0c6986` odnosi się do **recovery coordinator**, nie do początkowego binarium próby #5. Hash koordynatora zapisany przy uruchomieniu continuation to `ba864e8acb4ee0b53f0e47716b44e0dfc6ca9e62d3f29a3ffe9e0550017f40e5`. Kontynuacja korzysta z ulepszonego koordynatora, zachowując istniejący run i izolowaną kopię zadania.

Podczas tego audytu bieżąca ścieżka repozytorium `target/debug/antislop` miała już inny hash, `467615b8885a919d9dca1c713fd8b61a9b37834de9d0444582d9a87af6730610`, po kolejnym buildzie. Nadpisana ścieżka nie jest immutable dowodem zawartości wcześniej uruchomionego procesu. Hash continuation jest więc zapisem launch journal, a nie niezależnym odczytem zachowanego executable. Brak zachowanego immutable początkowego executable uniemożliwia retroaktywne udowodnienie jego hashy na podstawie aktualnej ścieżki.

Źródła koordynatora w bieżącym repozytorium różnią się od wcześniejszej izolowanej kopii m.in. rendererem promptów, obsługą diagnostics/recovery i komendami kontynuacji. Późniejsze testy w izolowanej kopii weryfikują jej wygenerowaną zmianę `src/main.rs`; nie dowodzą automatycznie przetestowania wszystkich późniejszych zmian koordynatora. Przyczyna pierwotnego zakończenia CLI pozostaje nieznana: stary adapter nie zachował wystarczających diagnostics. Duży payload jest zaobserwowanym czynnikiem, nie dowiedzioną przyczyną błędu.


## Update: v5 continuation preparation

The v5 context renderer was checked offline against frozen `attempt-6-blocked-run.json`; it retains complete required proof closure and all 124 expected checks. Its measured payload and exact hashes are recorded in `payload-audit.md`. The renderer's final source SHA-256 is `56411271243183726651fccffecef8d1d1adf3ee7c17e2e12857d8636f3f011a`. A coordinator retained before the final escaped-citation fix must not be described as containing that fix.

Subsequent live progress is outside that frozen snapshot. The root operator reported that the v4 Execution 6 verifier completed with ChangesRequested for actual missing performance baseline/scaling/memory/tail evidence (R071/R073/R079), and the driver proceeded to Execution 7. The operator later stopped the owned Execution 7 verifier and explicitly reconciled the stopped turn before preparing a replacement coordinator. These are attributed operator reports, not a new independent database observation by this audit. Supplemental input probes prove their observed input cases; they do not fabricate or waive benchmark requirements.

No v5 native turn or final Completed acceptance is demonstrated here. Continuing the existing run requires fresh actual evidence and successful independent review through the unchanged gates. This audit did not send native signals, invoke model CLIs, or mutate live run state.
