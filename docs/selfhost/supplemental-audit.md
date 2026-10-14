# Audyt rzeczywistych supplemental checks

Audyt read-only 2026-10-13 dotyczy tego samego runu `c77f6677-f800-483d-bc8a-f04d43b89b10`, nadal revision 1. Sprawdzono zapisane `after-supplemental-v2-run.json`, observations/argv v2, recovery journal, retained coordinator i transakcyjnie spójne odczyty SQLite przez `mode=ro`. Nie uruchamiano native CLI, nie modyfikowano runu ani aktywnej kopii. Harnessy odtwarzające scope/prompt działały wyłącznie na odczytanych snapshotach poza aktywną kopią.

## Zatwierdzony literalny argv

`supplemental-probe-argv-v2.json` jest identyczny z argv zapisanym w source rzeczywistego runner receipt. Ma postać `[python3, -I, -c, CODE, workspace]`; cały CODE jest częścią zatwierdzonego argv, a nie odwołaniem do później zmiennego skryptu.

- CODE SHA-256: `9343b31e371943ea39642a85bfc703668a21b01d5a95f60c42947ba2d6d1c9c7`
- argv SHA-256: `51a09d6a83886222009e7621b0f6380741f6297439f5388011356d660fd603a2`
- approval action: `check:51a09d6a83886222009e7621b0f6380741f6297439f5388011356d660fd603a2`
- approval scope: `3fb1ed925a2023f2eea793ab641f72059b545e26e7ec050b46b219c8d7438b5e`
- actor zapisany w approval: `local_user`

Scope przeliczony przez `Run::check_scope_hash(argv)` na właściwym zatrzymanym stanie Execution 5 był identyczny z approval. Analogicznie potwierdzono wcześniejszy v1: argv hash `d2967dd2b27eb387d604a8b6848da2f0ef74c8f41d97512f26f91e7833aaf5bd` i scope `ac168009decefe8789332ed90a2894ddc25957455d8a1aa7c7b2e40315c2fbc0`. Nie przenoszono stage approval na nowy niezatwierdzony command.

## Rzeczywisty runner i source binding

V2 receipt `2375b7f3-e1e9-43a4-851b-54995094663a` ma success=true, wrapper exit_code=0, stderr pusty i truncated=false. SHA-256 całego zapisanego output jest poprawny:

`bb406138c3ea9b3a17d5e1fe0ad6f1db8ad34d51b566532a85e1ac7278ea8d08`

Zapis `runtime:supplemental-check-inputs` potwierdza before_hash = after_hash = current input hash:

`ab12b49db6afdc59a86ea1d280b961651b6ed63640833c699849ff39c807f297`

Zapis ma unchanged=true, success=true, poprawny output hash i właściwy argv hash. Rzeczywiste pinned files odpowiadały temu snapshotowi; w porównaniu z początkowym `source-hashes.json` jedyną zmianą źródłową pozostał `src/main.rs`. Nie zmieniono checker/spec/requirements, aby uzyskać zaliczenie.

Osobny `supplemental-observations-v2.json` jest identyczny z JSON stdout zapisanym we wrapper receipt. Plan obserwacji odnosi się do rzeczywistego artefaktu Execution 5:

`99699d80f3f6992a92e6b8235302338c1c9b5d8290870ac2d2c99e850937bd05`

## Zakres i granice obserwacji

V2 zawiera 24 wykonane przypadki, powiązane z 19 applicable category labels z input-domain modelu. Nie było assertion_failure, timeout ani launch_error. Zweryfikowano długości i SHA-256 wszystkich 47 capturable stdout/stderr streams, odtwarzając oryginalne bajty z zapisanych Base64.

Obserwacje obejmują m.in. exact version, brak/pusty/numeric/Unicode/niepełny/neighbor/8192-byte verb, niewłaściwe UTF-8 także w późniejszym argv, duplikaty, version/help kolejność, positional `list --version`, ignorowaną ścieżkę po version, DEVNULL EOF, fd 0 zamknięty przed exec, stdin bytes i broken stdout. Zapisano rzeczywisty efekt positional state directory: wcześniej nie istniał, następnie został utworzony wewnątrz prywatnego cwd, z exit 1 i pustym stdout. Dla untrusted argv `sentinel_created=false` sprawdzono w rzeczywistym prywatnym cwd.

Broken stdout nie ma capturable strumienia bajtów: pipe nie miał czytelników. Zapis uczciwie oznacza stdout jako unobservable, zamiast przypisywać mu hash pustego output. Rzeczywisty proces zakończył się exit 101 ze stderr zawierającym `failed printing to stdout: Broken pipe`. Nie jest to twierdzenie, że udany zapis na zamknięty pipe był możliwy.

Fd 0 zamknięto przed exec; nie dowiedziono jego stanu po inicjalizacji Rust/OS, która może go sanitize/reopen. Długi argv był reprezentatywnym launchable 8192-byte przypadkiem, a nie pomiarem maksymalnego OS ARG_MAX. Obserwacje nie są benchmarkiem ani pełnym dowodem dla wszystkich argv. Nie uzasadniają zmyślonych latency/RSS/baseline wartości. Dopuszczalność scope-specific performance/security N/A wynika z wcześniej istniejącego validatora i wymaga osobnej niezależnej oceny źródeł; probe niczego takiego sam nie zatwierdza.

Probe sprawdził przed i po swoich przypadkach task binary SHA:

`9417f7e56dc656bd29d11f492cdb1541f82e8c382dbcd29f530333fc64be64f4`

Przy późniejszym odczycie ta zmienna ścieżka build output miała już inny digest, mimo niezmienionych źródeł/pinned inputs. Obserwacje odnoszą się do zapisanego, sprawdzonego pre/post binarium; nie dowodzą wieczystej byte identity każdej późniejszej przebudowy. Samo wcześniejsze binarium tasku nie zostało tu zachowane jako immutable executable.

## Zachowany coordinator i nowy rzeczywisty prompt

Retained coordinator `/tmp/antislop-selfhost-00wizn49/coordinators/supplemental-v1/antislop` ma SHA-256 zgodny z manifestem:

`51ba3443ab7c85811c863ddb4c27ed5afbd05faf6bcf51eea687d181febcb832`

Zweryfikowano także wszystkie wpisy source_hashes manifestu względem bieżących odpowiadających plików repozytorium. To mocniejszy retained artifact niż wcześniejsza nadpisywana ścieżka koordynatora. Nadal nie oznacza, że pierwotne i późniejsze etapy wykonywało jedno niezmienne binarium.

Nowy pre-send executor widok Execution 6 odtworzono z after-supplemental-v2 state oraz faktycznych approvals/evidence do pre-turn snapshotu. Odtworzony orchestration context był identyczny z trwałym snapshotem; oba prepared hashes były identyczne z SessionLedger:

- turn: `e3cf5993-acd5-4afd-bf1d-fa7be84e0394`
- native session: `01a1351a-3664-73a3-a535-b2fdfeb1de9a`
- prompt: 668984 B; schema: 15092 B; razem: 684076 B
- prompt SHA-256: `2c594ef7319b923e5650b15caa7000a12a164e961d35446bd7b51f7376871321`
- schema SHA-256: `9fd317ae9c6f6d3eeee144821fda74cc5b55e49d84e6cea03e45176297b6f1c4`

## Ostatni oceniony wynik

Końcowy eksport `attempt-6-blocked-run.json` rozstrzyga dwa wcześniejsze odczyty wykonane w różnych chwilach: Execution 6 jest **Blocked**, executor zakończył się jako Completed (native `01a1351a-3664-73a3-a535-b2fdfeb1de9a`, 187146 input tokens / 6688 output tokens), natomiast główny verifier zakończył CLI exit 1 (native prefix `01a1351d`) bez successful completed turn. Wcześniejsza migawka nie wystarczała do przypisania roli nieudanego turnu; nie przypisuję tej awarii executorowi. Liczba reviews nadal wynosiła 6, a ostatni review dotyczył Execution 5. **Nie ma nowego review potwierdzającego zamknięcie input-domain/performance findings ani końcowego Completed.** Nie przypisuję przyczyny temu rozmiarowi promptu bez dodatkowej diagnostyki.

Rzeczywiste supplemental checks zakończyły się poprawnie i ich scope/data bindings są spójne. Dalsza akceptacja wymaga skutecznego nowego turnu, niezależnego review i wszystkich aktualnych bramek. Audyt zakończono bez dalszego pollingu lub zmiany stanu.
