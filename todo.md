TODO list:

- [x] Pierwszy krok pracy z antislop po wyslaniu zadania jest wywiad z uzytkownikiem. Nastepnym krokiem jest napisanie planu. Widok grafu agentow na tym etapie powinien byc ukryty.
- [x] Plan stwozony dla danego zadania wyslanego przez uzytkownika powinien zostac pokazany na calym ekranie jak dokument w ktorym uzytkownik porusza kursorem miedzy liniami tekstu i jest w stanie skomentowac dowolna linie planu. Pod tekstem na stalej belce dolnej dostepne przyciski. Accept / Reject / Comment whole plan.
- [x] Komentarze dodane do planu powinny zostac rozwiazane lub odpowiednio zaadresowane.
- [x] Plan po rozwiazaniu wszystktkich komentarzy uzytkownika powinien zostac zamieniony na liste zadan do wykonania, zadania powinny miec okreslone zaleznosci i informacje o mozliwosci wykonania rownolegle z innymi zadaniami.
- [x] Uzytkownik w kazdej chwili powinien miec mozliwosc wgladu w plan.
- [x] Mozna sie zastanowic nad mozliwoscia przypisywania zadan do konkretnych sekcji planu i nastepnie fragment planu ktory zostal wykonany moze zostac podkreslony w odpowiedni sposob.
- [x] Agenci powinni miec mozliwosc pracy rownleglem tam gdzie ma to sens.
- [x] Argument aplikacji --project nie jest potrzebny. Zadne ze znanych narzedzi nie obsluguje takich paramterow. Katalogiem roboczym jest katalog w ktorym zostal uruchomiony antislop czyli najczesciej katalog glowny projektu. Nie trzeba sprawdzac czy jest to katalog glowny projektu. Katalog w ktorym zostal uruchomiony antislop jest katalogiem dostarczonym przez uzytkownika.
- [x] Trzeba dodac mechanizm akceptacji i zaufania danego katalogu roboczego przy pierwszym uruchomieniu w danym katalogu tak jak to dziala w innych narzedziach jak np codex.
- [x] UI powinien wyswietlac toggle dla trybu autonomous albo supervised
- [x] Komendy /pause /resume /continue /cancel takze powinny zostac obsluzone jak przyciski UI.
- [x] Podglad sesji w szczegolach agenta powien emulowac terminal procesu w ktorym jest uruchominy dany agent, tak zeby uzytkownik mial wglad we wszystkie operacje i komunikaty jakie przetwarza dany agent.
- [x] dodac obsluge uruchamiania aplikacji agentow za pomoca komend udostepnionych przez inne platformy. Przyklad: chcialbym moc skonfigurowac apliakcje tak ze orkiestrator jest dziala jako domyslny codex, a workerzy i werfikatorzy sa startowani przy pomocy komendy np omlx launch codex, albo ollama launch codex.
- [x] Kazdy tekst wyswietlony w UI powinien zostac przetlumaczony na Angielski.
- [x] Trzymanie effort map w konfiguracji nie ma sensu. Apliakcje maja jedna drabine skalowania dla myslenian i trzeba je zahardkodac.
- [x] Czesc agentow z dostepnej puli predefiniowanych agentow powinna byc wymagana, tak aby zawsze zapewnic najlepsza jakosc i pozostawic kod w lepszym stanie niz byl, oraz zapewnic ze wszystkie wymagania zostaly spelnione.
- [x] funkcjonalnosc eksportu powinna zostac usunieta.
- [x] UI musi wspierac myszke / touchpad.
- [x] Dodac mozliwosc pracy nad kilkoma projektami na raz.
- [x] okno z planem projektu nie zawija wierszy i nie da sie przeczytac dlugich linii tekstu. Trzeba dodac word wrap.
- [x] Agent ktory ma problem powinien zostac oznaczone kolorem czerwonym
- [x] Aplikacja powinna udostepnic mechanizm odblokowywania krokow miedzy agentami kiedy jest jakis problem ktory bolkuje ppstep prac.
- [x] W widoku sesji w szczegolach agenta nie widac wszystkich wiadomosci wyslanych do i od agenta. Brak np wpisow reasoning. Terminal takze nie wyglada jak originalna aplikacja.
- [x] W widoku szczegolow agenta powinny wyswietalc sie zakladki ktore maja sens dla danego agenta.
- [x] Agenci w widoku grafu powinni miec zaimplementowany color coding tak zeby mozna bylo ich latwo rozroznic.
- [x] Brakuje uzycia ikonek, loaderow tak zeby uzytkownik mogl wizualnie odroznic rodzaje agentow i czy aktualnie pracuje, czekaja, maja problem itp. Tak samo stan orkiestratora powinien byc aktualizowany w oknie czatu bo teraz po wyslaniu wiadomosci brakuj jakiegokolwiek feedbacku do uzytkownika czy aplikacja cos robi.
- [x] W widoku grafu nie chce uzywac ramek wokol kart agentow, samo tlo karty powinno ich odroznic od tla canvasa
- [x] orkiestrator powinien reagowac na zmiany stanow w grafie i informowac uzytkownika o pistepach zakonczonych rund w pustaci krtokich komunikatow. problemy ktore boluja pistep rowniez powinny byc raportowane do orliestratora.

Niedokończone względem design.md:

Ledgery i bramki jakości (§3, §6.4-6.7, §7.6, §10.1, M2Q):

- [ ] RequirementLedger i PlanItemLedger w runtime: atomowe wymagania, statusy (identified..verified/stale/superseded_by_user), dowody, rodzic/dzieci (§6.6).
- [ ] spec_revision na runie oraz unieważnianie zależnych zadań, raportów, modelu i dowodów po zmianie wymagań, modelu lub checklisty (§6.3, scenariusz 31).
- [ ] ContextModel, rejestr Unknown, SolutionModel, SimulationCase, TraceabilityMap jako rekordy z bramkami context_ready, plan_ready, solution_selected, requirements_complete egzekwowanymi przez runtime (§6.4-6.5).
- [ ] OptionsAnalysis i SolutionDecision jako ustrukturyzowane rekordy (warianty, kryteria, odrzucenia, wrażliwość, warunki ponownego otwarcia) (§6.7).
- [ ] RiskCoverage: obowiązkowa pozycja RISK-Rxxx dla każdego wpisu katalogu ryzyk z §19, z wykonawcą, weryfikatorem i uzasadnieniem wyłączeń (§7.6).
- [ ] ChecklistDefinition jako wersjonowany katalog z content_hash: wspólne V01-V08 i rozszerzenia każdej roli weryfikującej (§7.6).
- [ ] Runtime odrzuca raport z pustą pozycją checklisty, brakującym ryzykiem, nieaktualnym dowodem lub niedozwolonym autorem (scenariusz 30).
- [ ] Rekordy InputDomainMatrix, ChangeRisk, DependencyDecision, LicenseAssessment, RefactorReport zamiast samych obligacji w promptach (§10.1, §12.2).
- [ ] ComplianceAudit: końcowe porównanie pełnego źródła z rejestrem wymagań, blokada ukończenia runu (§6.6, scenariusze 27-29).
- [ ] Testy runtime zamiast samego promptu dla scenariuszy akceptacyjnych 11, 19-23, 34-40 (§18).
- [ ] Graf unieważniania dowodów i QualityGate (required_check_ids, input_hashes, invalidation_dependencies) (§10.1).

TUI (§8):

- [ ] Zakładki szczegółów agenta: Overview (ze scalona Session), Artifacts, Findings, Messages, Evidence, Checks. Brakuje Artifacts, Findings i Evidence jako osobnych zakładek (§8.4).
- [ ] Widoki runu z czatu/palety: Requirements, Options, Risks, Quality gates (pokrycie, dowody, pominięcia, różnice rewizji) (§6.6, §8.4).
- [ ] Filtry relacji w grafie (supervises, depends_on, verifies, communicates_with) oraz podświetlenie krawędzi komunikacji z czasem i licznikiem (§8.3).
- [ ] Pełny zestaw 12 statusów węzła (queued, starting, running, waiting_agent, waiting_user, verifying, changes_requested, paused, blocked, failed, completed, cancelled) z tekstem/symbolem i opcjonalnym kolorem (§8.3).
- [ ] Grupowanie po etapach/modułach i zwijanie zakończonych gałęzi w dużych zespołach (§8.3).
- [ ] Komentarze do każdego elementu panelu agentów: relacja, sesja, tura, wiadomość, dowód, pozycja checklisty, bramka (§8.5).
- [ ] Nagłówek z run i stage oraz pojęcie etapów (Intake..Delivery) w stanie runu (§6.1, §8.1).
- [ ] Wybór sposobu dostarczenia wiadomości do aktywnego agenta: natywne sterowanie, kolejka na następną turę, przerwanie i wznowienie; UI pokazuje dostępny wariant (§8.4).
- [ ] Komenda `antislop ui` ustalona w §8.2 (obecnie jest `antislop chat` i uruchomienie bez argumentów).

Runtime, daemon, adaptery (§7, §9, §10, §13, §14):

- [ ] request_id i klucz idempotencji dla zmian stanu w protokole daemona (§9.3).
- [ ] Jeden aktywny klient sterujący i klienci podglądu; konflikt sterowania nie może być last-write-wins (§9.3).
- [ ] Kontrakt adaptera: probe z capabilities, send z delivery_mode i receipt, interrupt z receipt rozróżniającym żądanie od potwierdzonego zatrzymania (§9.4).
- [ ] Codex przez app-server (JSON-RPC, steering) zamiast exec --json (§9.5).
- [ ] Uruchomienie i weryfikacja adaptera gemini na prawdziwym binarium, domknięcie macierzy wersji M0 (§16).
- [ ] Profile uprawnień read-only, workspace-write, restricted-execution, extended-access; blokada startu sesji, gdy ograniczenia nie da się wymusić (§13).
- [ ] Redakcja sekretów w logach i artefaktach, prawa plików, polityka retencji (§13).
- [ ] Zużycie tokenów oznaczone reported / estimated / unavailable (obecnie tylko reported lub brak) (§14).
- [ ] Natywne subagenty CLI wyłączone albo zarejestrowane jako zarządzane węzły; agy i gemini nie mają przełącznika (§9.5).
- [ ] Szablony zespołów (small-change, feature, refactoring, security-fix, greenfield, research, document, analysis) jako wersjonowana definicja w kodzie, nie tylko opis w instrukcjach orkiestratora (§7.3).
- [ ] Wersjonowane wbudowane role i szablony utrwalane przy starcie runu (§9.6).
- [ ] Resume na poziomie zadania oraz ponawianie zadań w stanie waiting (§14).
- [ ] Stan delivery_unknown i uzgadnianie z sesją przy niepewnym dostarczeniu wiadomości (§10.4).
- [ ] Zdarzenie i uzasadnienie przy każdej zmianie składu zespołu (§7.3).
- [ ] Walidacja wszystkich potrzebnych profili przed startem zespołu; brak CLI daje jawną propozycję zamiany lub blokadę (§7.4).

Workspace (§11):

- [ ] Projekt bez gita: utworzenie inicjalnego snapshotu/repozytorium przez runtime (design §11.1; obecnie writer prosi o git init).
- [ ] Lease plików i osobne zasoby (porty, bazy, cache) dla równoległych worktree (§11.1).
- [ ] Research, dokumenty i analizy bez gita: wersjonowane katalogi wejść/wyjść i artefakty z hashami (§11.2).

Zgody i tryby (§6.2):

- [ ] Supervised: pytanie o zgodę przy każdej rundzie poprawek (obecnie pierwsze 3 rundy bez pytania).
- [ ] Żadnego automatycznego otwierania rundy poprawek po odrzuceniu wyniku w trybie supervised, zgoda per etap i per runda (§6.2).

Pozostałe:

- [ ] Historia runu mierząca czas do zaakceptowanego wyniku, czas pracy użytkownika, liczbę rund, blokad, konfliktów i zmian zakresu (§15).
- [ ] Decyzja o licencji i dystrybucji Architect (§20, pkt 4).

- [x] zunifikowac workera z weryfikatorem w jednym nodzie na grafie
- [x] zunifikowac zakladke overview z session
- [x] Globalny fallback konfiguracji, gdy brak projektowego antislop.toml (t2).
- [x] Edytor dowolnej opcji konfiguracji w TUI i zastosowanie na zywo w daemonie (t3/t7).
- [x] Zaznaczanie i kopiowanie tekstu sesji myszka (t8).
- [x] Odswiezanie otwartych Checks po zmianie rundy i zdarzeniach check.
- [x] T14: sprawdzenie globalnego zaufania przed otwarciem stanu daemona; brak migracji zaufania z bazy projektu; bezpieczny plik tymczasowy rejestru.
- [x] T14: ograniczenie utrwalanych bledow tur i redakcja znanych sekretow ze srodowiska (nie obejmuje calego transkryptu).
- [x] T13: lokalne uniewaznianie Checks; odczyt jednego zadania bez dekodowania calego planu i ponownego odczytu dla freshness token.
- [x] T13: cache rekordow i zawijania transkryptu; przeglad zintegrowanego kodu.
- [x] T13: lekki endpoint task_checks bez fingerprintu Git; cache layoutu grafu i przycinanie tras do viewportu.
- [x] T14: integracja lancucha b6356f1 z baza main d050f2c, przeglad nowego kodu i pelna bramka: check, fmt, clippy -D warnings oraz 473 testow (bez pominiec).
- [x] T14: usuniety lokalny test routingu zgodnie ze zmiana uzytkownika; antislop.toml bez zmian.
- [x] T14: SetConfig porownuje rewizje pod blokada; clipboard ma limit 64 KiB; test przerwania procesu przed i po publikacji konfiguracji.
- [x] T14: przeglad kontraktow i schematu 5→6; regresja profilu nowego verifiera w tej samej rundzie; surowy protokol odrzuca brakujaca i stara rewizje.

- [x] Dostarczanie wiadomosci do wszystkich rol, ograniczone ponowienia, ochrona przed petlami odpowiedzi; poczta nie blokuje schedulerow planu.
- [x] Bezpieczne sprzatanie osieroconych plikow konfiguracji z ochrona aktywnych zapisow.

- [x] t8: porównano zapisane wyniki t4/t5/t6/t7; konflikt details.rs rozstrzygnięty z zachowaniem helperów i obu zachowań, bez końcowych zmian src/tests. Macierz sześciu wymagań i ręczne kontrole terminala: docs/lean-core.md, sekcja „Bramka integracyjna t8”.
- [x] t8: powtórzona końcowa bramka: fmt, clippy -D warnings, check i pełny cargo test --no-fail-fast — exit 0; 496/496, 0 pominiętych. Wszystkie 31 wcześniejszych awarii teraz OK; log /tmp/antislop-t8-final-verification.log. Dowody wykonania sześciu wymagań w docs/lean-core.md.
- [ ] t8: ręczna kontrola w prawdziwym terminalu: zaznaczanie/kopiowanie OSC 52, /config z zapisem na żywo, pojedynczy węzeł zadania i Overview, Checks przez kilka rund.
