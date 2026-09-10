# Store listing text, German

The German half of `store-listing.md`, one file per language. The headings are
that file's headings and stay in English, because `fenster`'s parser reads both
files the same way; only what sits under them is German. Only the fields a store
shows a reader are here.

**Terminology is the application's own, out of `po/de.po`** — a listing that
calls a thing something the window does not teaches the customer a word the
product has no use for. Where the catalogue has a term, it wins.

**German runs longer than English.** Run `fenster/check-listing.ps1` on this file
after any edit to either language rather than trusting a translation to fit.
## Short description

Die Nutzlast eines Slipcase-Containers in der Anwendung öffnen, die diesen
Dateityp ohnehin schon öffnet — und beim Speichern wird Ihre Änderung in den
Container zurückgeschrieben.

---

## App features

Up to twenty, each within 200 characters. Four-space indented, one per line,
which is what the fleet's listing parser reads.

    Doppelklick auf einen .slpc-Container, und das Dokument darin öffnet sich in der Anwendung, die Sie für diesen Dateityp ohnehin benutzen.
    Bearbeiten und speichern wie sonst auch. Die Änderung wird in den Container zurückgeschrieben; nichts muss exportiert und wieder eingelesen werden.
    Kein Fenster zu erlernen: ein Symbol neben der Uhr beantwortet, ob Ihre Arbeit dort ist, wo sie hingehört, und sein Menü nennt die offen gehaltenen Dateien.
    Endet eine Anwendung oder der Rechner, während ein Dokument offen ist, liegt die Änderung weiter auf der Platte und geht beim nächsten Mal in ihren Container zurück.
    Gefragt werden Sie nur, wenn der Container sich ebenfalls änderte — der eine Fall, in dem nur Sie antworten können.
    Kam ein Container aus dem Internet, trägt auch die entpackte Kopie diese Markierung, damit die öffnende Anwendung vorsichtig bleibt.
    Eine Nutzlast, deren Inhalt ein Programm ist, während ihr Name ein Dokument verspricht, wird abgelehnt, bevor etwas auf die Platte gelangt, und Sie erfahren warum.
    Geöffnet werden überhaupt nur gewöhnliche Dokument- und Bildtypen.
    Welche Dateitypen geöffnet werden dürfen, ist per Gruppenrichtlinie einstellbar und festsetzbar; eine ADMX-Vorlage ist separat erhältlich.
    Computerrichtlinie schlägt Benutzerrichtlinie, Benutzerrichtlinie schlägt die eigenen Einstellungen, und eine Sperrliste schlägt alles.
    Keinerlei Netzwerkverbindung. Es sammelt nichts und sendet nichts irgendwohin.
    Eine unterstützte Befehlszeile, kein Debug-Hilfsmittel: einen Container öffnen, auflisten was offen ist, und Zurückgelassenes wiederherstellen.
    Das Containerformat ist eine offene Spezifikation, veröffentlicht auf slipcaseformat.org, und ein Container ist ein gewöhnliches ZIP-Archiv.

---

## Description

Ein Slipcase-Container ist eine `.slpc`-Datei: eine einzelne Datei, die ein
Dokument zusammen mit einem Datensatz hält, der es beschreibt. Slipcase Open
ist das, was geschieht, wenn Sie darauf doppelklicken.

Das Dokument darin öffnet sich in der Anwendung, die Sie für diesen Dateityp
ohnehin benutzen — Ihrem PDF-Betrachter, Ihrer Tabellenkalkulation, Ihrem
Texteditor. Bearbeiten Sie es dort und speichern Sie wie sonst auch, und die
Änderung wird in den Container zurückgeschrieben. Es gibt keinen zusätzlichen
Schritt zu merken und nichts zu exportieren und wieder einzulesen.

**Es gibt kein Fenster zu erlernen.** Slipcase Open setzt ein Symbol neben die
Uhr, solange es ein Dokument offen hält, und die Farbe dieses Symbols ist die
ganze Oberfläche: sie beantwortet fortwährend eine einzige Frage, nämlich ob
Ihre Arbeit dort ist, wo sie hingehört. Sein Menü nennt die Dateien, die es
offen hält, und erklärt die Farbe, wenn es etwas zu erklären gibt.

**Ihre Arbeit geht nicht verloren, wenn etwas schiefgeht.** Endet eine
Anwendung oder der Rechner, während ein Dokument offen ist, liegt die Änderung
weiterhin auf der Platte, und Slipcase Open schreibt sie beim nächsten Lauf in
ihren Container zurück. Gefragt werden Sie nur in dem einen Fall, in dem Fragen
redlich ist — wenn der Container sich ebenfalls geändert hat, sodass nur Sie sagen können,
welche Fassung gemeint war.

**Es ist vorsichtig damit, was es öffnet.** Kam ein Container aus dem Internet,
sagt die entpackte Kopie das auch, damit die Anwendung, die sie öffnet, die
Datei mit der Vorsicht behandelt, die sie allem von außen entgegenbringt. Eine
Nutzlast, deren Inhalt ein Programm ist, während ihr Name ein Dokument
verspricht, wird rundheraus abgelehnt, bevor etwas auf die Platte gelangt, und Sie erfahren,
warum. Und
geöffnet werden überhaupt nur gewöhnliche Dokument- und Bildtypen.

**Für Administratoren.** Welche Dateitypen geöffnet werden dürfen, ist per
Gruppenrichtlinie einstellbar und festsetzbar; eine ADMX-Vorlage ist separat
erhältlich. Computerrichtlinie hat Vorrang vor Benutzerrichtlinie, diese vor
den eigenen Einstellungen eines Benutzers, und eine Sperrliste schlägt alles.
Die Anwendung sagt das in ihrer eigenen Oberfläche, wenn Einstellungen
verwaltet werden, damit eine Ablehnung sich als Entscheidung liest, die jemand
getroffen hat, und nicht als Software, die sich unvorhersehbar verhält.

Slipcase Open stellt keinerlei Netzwerkverbindung her. Es sammelt nichts und
sendet nichts irgendwohin.

Eine Befehlszeile gehört dazu und ist eine unterstützte Schnittstelle, kein
Debug-Hilfsmittel: `slipcase-open` öffnet einen Container, listet auf, was
offen ist, und stellt wieder her, was zurückgelassen wurde.

Das Slipcase-Containerformat ist eine offene Spezifikation, veröffentlicht auf
slipcaseformat.org. Ein Container ist ein gewöhnliches ZIP-Archiv, sodass
nichts, was Sie hineinlegen, an diese Anwendung gebunden ist.

---

## Keywords

**Microsoft Store** (seven terms, which is the limit):

    slpc, Slipcase, Container, Nutzlast, öffnen, bearbeiten, Archiv

`slpc` first, for the reason the English file gives: somebody who has been sent
a file they cannot open searches for the extension. `Slipcase` stays as it is,
being a product name, and `Nutzlast` is the word the application's own
notifications use for the payload.

---

## Release notes

*Neu in dieser Version* im Microsoft Store, ein Text je Fassung, neueste
zuerst. Für 0.1.5 gibt es keinen: das war die macOS-Fassung und änderte nichts,
was einem Windows-Benutzer dieser Anwendung auffiele.

### 0.1.6

Deutsch. Auf einem deutsch eingestellten Rechner erscheinen die
Benachrichtigungen, die Schaltflächen darauf und die stehende Liste auf
Deutsch, und ebenso das, was `sessions` und `recover` ausgeben. Es gibt nichts
auszuwählen: Slipcase Open übernimmt die Sprache, auf die Windows bereits
eingestellt ist, und fällt für jede andere auf Englisch zurück.

**Was das Werkzeug über Ihre Dateien sagt, ist übersetzt; was es über sich
selbst sagt, nicht.** `--help` und der Einstellungsbericht bleiben englisch,
letzterer, weil er Pfade in einer Tabelle druckt, die auf die Breite englischer
Wörter gebaut ist, und weil er das ist, was man in einen Fehlerbericht einfügt.
