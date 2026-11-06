# Rozszerzona karta grafu i PTY — pomiary oraz walidacja

## Zakres i pochodzenie dowodów

Ten raport indeksuje końcowe testy i niezmienne wyniki MCP poprzednich zadań. Walidacje UI pochodzą z końcowego raportu karty w generacji g5 oraz z końcowego testu współdzielonego worktree. Bramka integracyjna raportowała czysty złożony checkout `ffa3d010411c7c544232490cc756f2f2beab1347`. Przygotowujący ten raport checkout jest osobną alokacją na `3305891a65801bfa3bd32edcf2c3ff7234ba6cc2`; przed utworzeniem raportu `git status --short` był pusty. Nie uruchamiałem ponownie testów w tej alokacji raportowej. Dlatego wyniki poniżej są przypisane do konkretnych zapisanych poleceń i dowodów, a nie przedstawiane jako świeże wykonanie w tym checkoutcie.

Wersje z walidacji: `rustc 1.98.1 (48a229cea 2026-09-01)`, `cargo 1.98.1 (797e8a9bc 2026-08-05)`. Raport wcześniejszej implementacji podaje `hologram 0.17.0`.

## Wymiary, szerokość i granice grafu

Test `expanded_card_content_measurements_cover_compact_near_full_and_wide_terminals` porównuje renderowane prostokąty karty i treści z poprzednim limitem szerokości dla tych samych wymiarów terminala. Pole `area` to szerokość × wysokość treści po odjęciu ramek i wspólnego wiersza selektora rundy.

| Terminal | Viewport grafu | Treść nowej karty | Dawna treść | Przyrost |
|---|---:|---:|---:|---:|
| 80×24 | 76×14 | 66×7 = 462 | 52×8 = 416 | +46 |
| 120×30 | 75×19 | 65×12 = 780 | 52×13 = 676 | +104 |
| 160×50 | 102×39 | 92×32 = 2944 | 52×16 = 832 | +2112 |

W każdej parze terminal ma identyczne wymiary przed i po; karta jest szersza i przyjmuje niemal całą szerokość viewportu. Finalne wyjście testu z pomiarami jest w dowodzie `537dfd75-39ee-496c-91bc-e9145318d330`. Dla typowego PTY na terminalu 80×24 wcześniejsza geometria viewportu 76×12 miała kartę 56×10 i treść 52×6 = 312 komórek. Pierwsza zmiana zwiększyła ją do 70×10, 66×6 = 396 komórek (+26,9%; dowód `1cd8584c-0082-454d-8ded-2700c6c33dd9`). W układzie końcowym wspólny selektor rund zajmuje dodatkowy wiersz: 66×5 = 330 komórek dla tej samej karty 70×10. Nie należy mieszać pomiaru 396 z późniejszym układem zawierającym selektor.

`graph_position_markers_follow_offscreen_nodes_and_resize` asercjami sprawdza pozycję wskaźników: na górnej granicy nad viewportem puste miejsce, a pod nim `↓`; po przewinięciu do dołu nad viewportem `↑`, a pod nim puste miejsce. Test `graph_position_marker_glyph` sprawdza wariant ASCII `^` i `v`; `ascii_replaces_box_drawing_marks_and_letters` sprawdza konwersję rysunku. Przy terminalu mniejszym niż 12×5 wskaźniki grafu są wyłączone. Bufor płótna pozostaje ograniczony do obszaru viewportu.

## Wybór karty, PTY i rund

Wybór elementu jest odrębny od rozwijania: `graph_enter_expands_compacts_and_transfers_the_only_expanded_card` naciska Enter, sprawdza rozwinięcie, porusza się strzałką i potwierdza, że rozwinięcie pozostaje na tej samej karcie; dopiero Enter na innym zaznaczonym węźle przenosi kartę, kolejny Enter ją zwija. Test pomiaru również rozwija kartę osobnym Enter przed odczytem geometrii.

Dokładny skrót wyjścia z PTY to **Ctrl+]** (Control + prawy nawias kwadratowy). Test `expanded_graph_pty_renders_the_owned_snapshot_and_routes_input` włącza PTY, wysyła zwykły `x` i sprawdza przekazanie wejścia, a następnie wysyła Ctrl+]. Asercje sprawdzają powrót do Details (`graph_tab == 0`), brak wysłania skrótu do PTY, brak zatrzymania widoku PTY, brak ponownego przejęcia fokusu po redraw, dalsze przełączanie do Results klawiszem `3` i nawigację grafu strzałką. Pomoc TUI i pasek karty również pokazują Ctrl+] jako skrót opuszczenia PTY.

Test `expanded_card_keeps_one_round_across_tabs_and_never_attaches_historical_pty` używa klawiszy aplikacji i renderuje zawartość czterech prawdziwych zakładek karty. Przecinek wybiera zachowaną rundę 1 bez Enter; pasek widocznie pokazuje `Round 1` i dostępny kierunek `. next`. Zakładki 1–4 pokazują odpowiednio Details, PTY, Results i Diff, a asercje potwierdzają, że wybór rundy zostaje zachowany przy zmianie zakładki. Dane Details pochodzą ze snapshotu rundy 1, a Results z dokumentu fixture rundy 1. PTY rundy historycznej pokazuje komunikat read-only i test potwierdza, że nie tworzy widoku ani procesu. Diff historyczny pokazuje „No historical diff is retained”; naciśnięcie `c` nie otwiera edytora komentarza i daje komunikat braku diffu. Gdy pojawia się runda live 3, przypięta historia nie przeskakuje: `.` przechodzi do rundy 2, kolejny `.` wraca do `selected_round == None`, czyli bieżącego śledzenia.

## Współdzielony i izolowany worktree

`options_worker_review_and_repair_rounds_share_one_checkout_in_role_order` (test integracyjny z fixture CLI) sprawdza kolejność i `cwd` ról. Opcje worker i reviewer, worker implementacji, reviewer odrzucający pierwszą zmianę oraz worker naprawczy i reviewer zatwierdzający używają tego samego checkoutu zadania. Pierwszy reviewer widzi plik `B` i odrzuca; worker naprawczy zmienia go na `A`, a reviewer zatwierdza. Test asercjami sprawdza wszystkie `cwd`, kolejność ról i zawartość pliku po obu recenzjach.

Test `read_only_continuation_observes_retained_checkpoint_and_new_project_inputs` sprawdza restart: po zamknięciu i ponownym otwarciu Core ścieżka checkoutu opcji jest identyczna, checkpoint naprawy nadal jest widoczny, a checkout niezależnego zadania ma inną ścieżkę. Test `writers_without_path_scopes_run_concurrently_in_separate_worktrees` uruchamia dwa niezależne writery współbieżnie; oba kończą zatwierdzone, mają różne `cwd` worktree i nie piszą bezpośrednio do projektu. W ten sposób oddzielnie sprawdzone są współdzielenie w ramach zadania, odtworzenie po restarcie i izolacja różnych zadań.

## Zapisane polecenia jakości i surowe dowody

Każdy poniższy identyfikator wskazuje niezmienny wynik MCP, który zawiera dokładne polecenie, kod zakończenia i wynik:

| Polecenie | Wynik | ID dowodu |
|---|---|---|
| `./cargo.sh fmt --check` | kod 0, brak wyjścia | `58aca67d-7f27-4e16-bda9-af3a1577281a` |
| `./cargo.sh check` | kod 0, zakończono profil dev | `e0161595-d658-4209-9b67-7eb6a31eb8e6` |
| `./cargo.sh clippy --all-targets -- -D warnings` | kod 0, zakończono profil dev | `9c4f6548-0235-4fa6-a9d6-8fa8ebf0dd7b` |
| `./cargo.sh test --test lean_core` | kod 0, 24 passed, 0 failed | `498d7437-a01f-4ed0-bd93-6990ef3072a3` |
| `./cargo.sh test --test lean_tui` | kod 0, 109 passed, 0 failed | `4812ff76-576e-4f81-a100-2a78a8429d7d` |

Zbiorczy surowy zapis tych samych pięciu kontroli z finalnego testu karty jest też w `1863c6d7-be08-45d9-ba8a-f99c1ab9cdde`; końcowy test współdzielonego worktree ponownie raportuje je w `0a5eb109-01a4-43ae-8ac7-011ddf80975d`. W tym ostatnim `lean_tui` ma 109/109, `lean_core` 24/24, `lean_options` 9/9; rustc i Cargo to wersje wymienione wyżej.

Dodatkowe dowody: rozmiary karty `537dfd75-39ee-496c-91bc-e9145318d330`; wybór rund i kontrole końcowe karty `1863c6d7-be08-45d9-ba8a-f99c1ab9cdde`; bazowy pomiar PTY `1cd8584c-0082-454d-8ded-2700c6c33dd9`; jedna współdzielona ścieżka dla wszystkich ról `0a5eb109-01a4-43ae-8ac7-011ddf80975d`; odtworzenie checkoutu po restartach `4ed91034-7801-472e-aaee-34c95d2c760a`; współbieżna izolacja zadań `272ef250-af9a-4efc-ab3b-5aea96461428`.

Surowy dowód `d5d96669-3bc2-45ab-9bf9-2ef0acef9bb8` podaje dokładne polecenie regresji rund `./cargo.sh test --lib expanded_card_keeps_one_round_across_tabs_and_never_attaches_historical_pty -- --nocapture` i wynik 1 passed; zapisuje też pełne `./cargo.sh test --test lean_tui` jako 109 passed, 0 failed. Dowód `1863c6d7-be08-45d9-ba8a-f99c1ab9cdde` zawiera dokładne końcowe polecenie testu PTY `./cargo.sh test --lib expanded_graph_pty_renders_the_owned_snapshot_and_routes_input -- --nocapture`, zakończone kodem 0 (1 passed).

## Błędy i ograniczenia zapisane w trakcie prac

- Początkowe `cargo.sh fmt` zakończyło się w zsh komunikatem `command not found`, ponieważ wywołano nazwę bez `./`. Użycie `./cargo.sh fmt` i końcowe `./cargo.sh fmt --check` przeszło.
- Pierwsza asercja wymiaru oczekiwała szerokości 72; rzeczywisty margines 3 dawał szerokość karty 70. Poprawiono oczekiwaną wartość, a test geometrii przeszedł.
- Pierwsza wersja testu PTY sprawdzała kontroler po redraw, który jest prawidłowo usuwany; asercje zmieniono tak, by sprawdzić flagę zatrzymania przed redraw i routing TUI po nim.
- Początkowe `./cargo.sh hologram review ...` nie zadziałało, bo `hologram` jest osobnym programem, a nie podkomendą Cargo. `hologram review --root . --quiet-if-clean` zakończyło się kodem 0 bez uwag.
- Pierwszy pełny test TUI wykrył, że ładowana karta mogła zastąpić niezwiązaną treść orchestratora; ograniczono render do pasującego snapshotu i końcowe `lean_tui` przeszło.
- W toku testu współdzielonego worktree `fmt --check` najpierw wykazał formatowanie, kompilacja testu zgłosiła `E0425` dla omyłkowej nazwy `worker_reviews`, a następnie test wykrył brak `cwd` recenzera w fixture. Wszystkie trzy przyczyny naprawiono; końcowa asercja wszystkich `cwd` i pełny test opcji przeszły.

Repozytorium nie przechowuje zrzutów ekranu PTY ani historycznych Diffów powiązanych z rundą. Te braki są celowo widoczne w testowanej karcie; nie są dowodem na zawartość historycznego PTY/Diff.
