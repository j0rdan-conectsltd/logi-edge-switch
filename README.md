# Logi Edge Switch

Sposta il mouse contro il bordo dello schermo verso l'altro PC: **mouse e tastiera Logitech passano da soli al canale Easy-Switch di quel PC**.

Il programma usa il protocollo HID++ di Logitech (feature `0x1814 Change Host`), cioè lo stesso comando che parte quando premi il tasto Easy-Switch. Funziona sia con il Bluetooth diretto sia con i ricevitori Logi Bolt e Unifying.

## Download

| Sistema | File | Note |
|---|---|---|
| **Windows 10/11** (x64) | [LogiEdgeSwitch-windows-x64.exe](https://github.com/j0rdan-conectsltd/logi-edge-switch/releases/latest/download/LogiEdgeSwitch-windows-x64.exe) | Un solo file, nessuna installazione. Al primo avvio SmartScreen può chiedere conferma: *Ulteriori informazioni* → *Esegui comunque*. |
| **macOS 11+** (Apple Silicon e Intel) | [LogiEdgeSwitch-macos.dmg](https://github.com/j0rdan-conectsltd/logi-edge-switch/releases/latest/download/LogiEdgeSwitch-macos.dmg) | Apri il .dmg e trascina l'app in *Applicazioni*. |

Tutte le versioni: [Releases](https://github.com/j0rdan-conectsltd/logi-edge-switch/releases).

### Primo avvio

1. Avvia l'app: le periferiche Logitech collegate compaiono da sole in *Periferiche*.
2. Clicca i monitor o scegli i bordi verso l'altro PC, poi **Salva configurazione** e **Avvia**.
3. Attiva **Avvia all'accesso** in *Impostazioni*: da quel momento parte da sola, già attiva.
4. Ripeti sull'altro PC.

Su **macOS**, la prima volta:
- se l'app non è firmata, macOS la blocca: *Impostazioni di Sistema → Privacy e sicurezza → Apri comunque*;
- concedi *Monitoraggio input* (serve per parlare con mouse e tastiera) e *Accessibilità* (serve per leggere e spostare il cursore), poi riapri l'app.

La configurazione è salvata in `%APPDATA%\Logi Edge Switch\config.json` (Windows) o `~/Library/Application Support/Logi Edge Switch/config.json` (macOS). Se accanto all'eseguibile c'è un `config.json`, viene usato quello (uso portatile, da chiavetta).

## Come funziona

```
   PC 1 (canale 1)                     PC 2 (canale 2)
 ┌──────────────┐                    ┌──────────────┐
 │           ──►│ bordo destro       │◄──           │ bordo sinistro
 │              │  → canale 2        │  → canale 1  │
 └──────────────┘                    └──────────────┘
```

1. Ogni PC esegue Logi Edge Switch con la propria configurazione.
2. Quando il cursore resta per `dwell_ms` (150 ms) su un bordo **esterno**, cioè un bordo oltre il quale non c'è un altro monitor dello stesso PC, il programma invia "cambia canale" a tutte le periferiche Logitech collegate.
3. In alternativa ai bordi, un monitor collegato a entrambi i PC può essere marcato "condiviso": entrandoci si passa all'altro PC e il cursore torna indietro, così al rientro non riscatta.

Protezioni contro i cambi involontari:
- gli angoli vengono ignorati (`corner_margin_px`);
- nessun cambio mentre tieni premuto un tasto del mouse, per esempio quando trascini una finestra per agganciarla al bordo;
- dopo un cambio c'è un periodo di attesa (`cooldown_s`), e il cursore deve staccarsi dal bordo prima di poter scattare di nuovo.

## Compilare dai sorgenti

Serve solo per sviluppare; per usarlo basta il [Download](#download). Serve Rust (https://rustup.rs). Su Windows servono anche i Visual Studio Build Tools (C++).

```bash
cargo build --release
```

L'eseguibile è `target/release/logi-edge-switch` (`.exe` su Windows). Su Windows basta `installa.bat`, poi `avvia.bat`.

Su **Linux** serve anche l'accesso a `/dev/hidraw*`, altrimenti devi avviarlo con sudo:

```bash
echo 'KERNEL=="hidraw*", ATTRS{idVendor}=="046d", MODE="0660", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/42-logitech-hidpp.rules
```

```bash
sudo udevadm control --reload && sudo udevadm trigger
```

Su **macOS**: Impostazioni di Sistema → Privacy e sicurezza → abilita *Monitoraggio input* e *Accessibilità* per `logi-edge-switch`.

## Configurazione

Il modo più semplice è l'interfaccia grafica: lancia `logi-edge-switch` senza argomenti.
- **Disposizione schermi**: clic su un monitor per passare tra "Questo PC" e "Altro PC canale 1/2/3" (monitor condiviso). Sotto scegli il canale per ogni bordo esterno.
- **Periferiche**: elenco, nuova ricerca e pulsanti per provare il cambio canale.
- **Avvia/Ferma** e **Salva configurazione** (le modifiche valgono subito, il salvataggio le rende permanenti).

Con `--start` parte già attivo e ridotto a icona (utile per l'avvio automatico). `-c file.json` usa un config diverso; senza, usa quello descritto in [Primo avvio](#primo-avvio).

Da riga di comando:

1. Collega le periferiche al PC e controlla che vengano trovate:
   ```bash
   logi-edge-switch --list
   ```
   L'output è simile a:
   ```
   - MX Master 3S (Bluetooth) - canale 1/3
   - MX Keys (ricevitore slot 1) - canale 1/3
   ```
2. Prova il cambio a mano (le periferiche dovrebbero passare al canale 2):
   ```bash
   logi-edge-switch --switch 2
   ```
3. Copia `config.example.json` in `config.json` e modificalo. In `esempi/` trovi le configurazioni pronte per 2 PC e per 3 PC.

| Campo | Significato |
|---|---|
| `rules[].edge` | `left`, `right`, `top` o `bottom`: il bordo verso l'altro PC |
| `rules[].monitor` | in alternativa a `edge`: `"x,y"` di un monitor condiviso con l'altro PC; entrarci fa passare a `channel` (le coordinate sono stampate all'avvio) |
| `rules[].channel` | canale Easy-Switch (1-3) di quel PC |
| `rules[].device_channels` | eccezioni per dispositivo, se la tastiera usa un numero di canale diverso dal mouse. Esempio: `{"MX Keys": 3}` |
| `devices` | filtro per nome: `["MX Master", "MX Keys"]`. Se è vuoto, usa tutte le periferiche |
| `dwell_ms` | per quanti millisecondi il cursore deve restare sul bordo |
| `corner_margin_px` | zona vicino agli angoli in cui non scatta nulla |
| `cooldown_s` | pausa minima tra due cambi |
| `ignore_while_dragging` | se è `true`, non cambia canale mentre trascini qualcosa |


## Avvio automatico

Su Windows e macOS basta l'interruttore **Avvia all'accesso** in *Impostazioni* (aggiunge l'app alla chiave `Run` dell'utente su Windows, crea un LaunchAgent su macOS).

- **Linux**: usa un servizio utente systemd, oppure crea una voce `.desktop` in `~/.config/autostart`.

## Note e limiti

- **Logi Options+ ha già "Flow"**, che fa una cosa simile e copia anche gli appunti. Se lo usi, disattivalo, perché i due sistemi andrebbero in conflitto. Questo programma serve se Flow non va con le tue periferiche o i tuoi sistemi, oppure se vuoi più controllo.
- Il programma comanda solo le periferiche **collegate al PC su cui gira**. Per questo deve essere installato su ogni PC.
- Dopo il cambio, il dispositivo impiega di solito 1-2 secondi a riconnettersi all'altro PC. È lo stesso tempo del tasto fisico.
- Le periferiche più vecchie che non supportano HID++ 2.0 o Easy-Switch non compariranno in `--list`.
