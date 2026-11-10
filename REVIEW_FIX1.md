# Review-Fixzyklus 1 — zur erneuten unabhaengigen Pruefung

Auftrag: ausschliesslich die Findings zum Stand `b8d1d44bf4e3a33d211d81e718e284a2760f835e` beheben. Worktree `.worktrees/hermes-omnitrade-offline-core`, Branch `hermes/offline-execution-core`. Kein Push/Merge/Deploy, kein Betriebsneustart, kein Zusatzstop und kein Sweep. Rust-Aenderungen bleiben im vorhandenen `cfg(test)`-Modul.

Fixcode: `c56106b` (Author Hermes); weiterhin unabhaengiges Review ausstehend, keine `[verified]`-Kennzeichnung.

## Aenderungen

- FakeExchange protokolliert unveraenderliche sequenzierte Request-/Response-/Error-Ereignisse mit paarweiser Identitaet. Response enthaelt tatsaechlichen Preis, gefuellte Menge, Status und Gebuehr; IDs sind nicht mehr konstant. Replay uebernimmt diese primaeren Ereignisse statt StatusWriter-Preise als Fills auszugeben. StatusWriter `last_order` bleibt ausdruecklich nur Entscheidungs-/Referenzpreisbeleg. Vom Guard abgelehnte Versuche erreichen die Exchange nicht und haben deshalb keine erfundene ExchangeAntwort.
- Nicht unterstuetzte Typen/Instrumente/ungueltige Requests und ungueltige injizierte Fills werden abgelehnt. Injizierter abweichender Fillpreis, Teilfill und ExchangeFehler getestet; Short-Gewinn/-Verlust, exakter Nullabschluss und Durch-Null-Wechsel mit unabhaengig festgelegten Zahlen getestet.
- Validator prueft jede Kerze, genaue int/nicht-bool/nichtnegative Counter und Deltas einschliesslich Hold, vollstaendige exakte Schemata, endliche Zahlen, Ereignissequenzen/Identitaeten/Typen, Menge und echte Fills. Signed-Inventory-Cost-Rechnung rekonstruiert Cash, kumulierte Fees, Position, Durchschnittseinstand und Equity aus Fillereignissen, statt gespeicherte Bilanzwerte als Wahrheit zu verwenden. Die eingefrorene Referenz bestimmt weiterhin Signale, Annahme/Ablehnung und Vergleichsfills.
- Alle Validatorbedingungen verwenden explizite Exceptions, keine unter `python -O` entfallenden asserts.
- Replay und Validator schreiben standardmaessig frische eindeutige Scratchordner. Export ist explizit (`OFFLINE_REPLAY_EXPORT` bzw. `--export`), atomar per Hardlink und ohne Ueberschreiben; gleicher Datentraeger erforderlich. Alte Referenzen und Belege bleiben unveraendert.

## TDD und echte Ergebnisse

Zuerst ausgefuehrte rote Kontrollen (Originalausgaben im Werkzeugtrace dieser Fixsitzung):
- `python offline-artifacts/test_verify.py`, auch `python -O`: alle sieben urspruenglichen Mutationen entgingen dem alten Gate (7 Subtestfehler je Modus): Hold-Counter, Hold-NaN, fills999, bool/negative Counter, filled_quantity99 und rejectPrice999.
- `cargo test ... offline_unknown_order_type_fails_closed`: rot, Limit wurde angenommen.
- `cargo test ... offline_actual_partial_fill_and_error_evidence`: rot, erhalten 100 statt injiziertem 101.
- Zusaetzlicher Referenzschema-Test zuerst rot: unbekanntes Ledgerfeld wurde nicht abgelehnt.

Neue Ausfuehrungsbelege liegen ausschliesslich in `offline-artifacts/review-fix1-final/`:
- `cli-tests.log`: **114 bestanden**, keine Fehler, einschliesslich sieben Offline-Tests und echtem Vollreplay.
- `workspace-tests.log`: **524 bestanden**, keine Fehler (Summe der Rust-Testzusammenfassungen einschliesslich Doc-Tests).
- `workspace-clippy.log`: `cargo clippy --offline --target-dir target-offline --workspace -- -D warnings`, Exit 0. Kein `--all-targets`-Clippy behauptet.
- `verify.log` und `verify-optimized.log`: je 1440 Kerzen, 206 Versuche, 107 angenommen, 99 abgelehnt, Position 0.30000000000000004, Equity 50.00923664999997; fuenf eingefrorene Datei-Hashes stimmen.
- `mutations.log` und `mutations-optimized.log`: je drei Python-Tests bestanden; jede der **32 Replaymutationen einzeln durch die echte CLI normal und -O abgelehnt**, zuvor jeweilige positive Kontrolle bestanden. Vier zusaetzliche Referenzmutationen ebenfalls abgelehnt. Keine Erwartungswerte aus neuen Outputs generiert.
- `rust-no-clobber.log`: zweiter Vollreplay mit bestehendem Export absichtlich Exit 101; Export unveraendert. Das ist eine erwartete Negativkontrolle, kein gruener Replaylauf.
- `concurrent-export.log`: zwei parallele Pythonexporte auf dasselbe Ziel; genau ein Exit 0 und ein Exit 1, vollstaendiges JSON nachgelesen.
- `run-results.json`: Exitcodes und SHA256-Inventar bestaetigen unveraenderte alte `hype-replay.json`, `comparison.json` und alle Fixturedateien.

Fehlgeschlagene neue Laeufe wurden **nicht ueberschrieben**: `review-fix1/verify.log` zeigt zu enges Referenzschema, `review-fix1-retry/verify.log` die unten genannte Digest-Diskrepanz. Beide Vollreplays sind separat erhalten. Das Schema benennt jetzt alle tatsaechlich vorhandenen Referenzmetadaten und prueft deren Typen/Annahmen.

## Nicht verifiziert / keine Paritaetsfreigabe

1. Mainnet-Dateninstrument `HYPE-USD_UM_XPERP-310523` und synthetisches Demo-Testinstrument `HYPE-USD_UM_XPERP-310801` sind **nicht identisch**. Diese Tests beweisen nur interne Konsistenz des benannten Simulationsvertrags.
2. Historische Risikouhr, Tageswechsel und Resetsemantik sind nicht kontrolliert. Die reale Loop-/RiskGuard-Uhr bleibt Wandzeit. Kein Nachweis historischer Risikoparitaet und keine Freigabe fuer laengere Historien.
3. **Zusaetzlicher offener Provenienzbefund:** `manifest.sha256` und `reconstruction.input_sha256` nennen `3ce032663f31a1673118af4c4be86b666a2d951c1becb46c31202603ba92184a`; die tatsaechlichen kopierten `candles.csv`-Bytes haben laut verifiziertem `hashes.json` `12bc1dcec41b6bf4da89060fe342e2e5efc027f569d95ab067720a1405d4389b`. Auch LF-Normalisierung erklaert dies nicht (2ff26ff3fde0966df3e239c6ac519bcfce08876ec5b5774f187e1e17703023a4). Referenzen wurden nicht veraendert. Der aktuelle Konsistenzvalidator verifiziert die fuenf aktuellen Fixturedateien sowie Uebereinstimmung der beiden historischen Metadaten, **nicht** die Identitaet des urspruenglichen Exports mit der kopierten CSV. Diese Provenienzkette bleibt offen; der fehlgeschlagene direkte Digestvergleich bleibt als roter Beleg erhalten.
4. Vollreplayvertrag bleibt Vollfill, Fee 0.0005, null Slippage/Funding, kein erzwungener Endabschluss. Teilfill-/Fehlerbeobachtung ist auf Fake-Traitebene getestet, nicht als Beweis fuer korrekte fortgesetzte Produktions-Risikobuchung nach einem Teilfill. Reale Margin, Rundung, externe Positionsaenderungen, Exchange-Ausfuehrung, Latenz und Profitabilitaet sind nicht verifiziert.
5. Atomic-Export-Konkurrenz fuer Python und bestehendes Rust-Exportziel wurden getestet; abgebrochene Prozesse und Cross-Volume-Export nicht injiziert. Cross-Volume schlaegt konstruktionsgemaess fehl statt zu kopieren.

## Reproduktion / naechster Schritt

Prozesslokal `CARGO_HOME`/`RUSTUP_HOME` auf vorhandene `.config/cargo`/`.config/rustup`, alle drei `TMP`/`TEMP`/`TMPDIR` auf den freigegebenen Hermes-Scratchpfad setzen. Keine Installation. `cargo test --offline --target-dir target-offline -p omnitrade-cli offline_ -- --nocapture` erzeugt einen neuen Scratchreplay und druckt `OFFLINE_REPLAY=<Pfad>`. `python offline-artifacts/verify.py <Pfad>` sowie `python -O offline-artifacts/verify.py <Pfad>`; fuer Mutationen `OFFLINE_TEST_REPLAY=<Pfad>` setzen und `python offline-artifacts/test_verify.py` ausfuehren.

**Naechster Schritt:** Parent beauftragt einen frischen Reviewer mit dem Fixcommit, diesem Vertrag, `review-fix1-final/` und einer eigenen Einzelmutation. Provenienz-Diskrepanz und historische Risikouhr bleiben explizite Sperren gegen eine umfassende Ausfuehrungsparitaets-/Optimierungsfreigabe. Kein Merge oder Sweep aus diesen gruenen Konsistenztests ableiten.
