"""Freeze the diagram embedded in suimon's 2013-05-31 game article."""

import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import subprocess


BASE = Path(__file__).parent
SOURCE = Path("/private/tmp/sekirei-suimon-t01.html")
OUT = BASE / "corpus-pt71b.json"
VERIFY = Path("/private/tmp/sekirei-pt65-verify-moves")
SOURCE_SHA = "dab9e4df7198f902e038a34d7e8676ca0ceb78949dec1fc049b1de0a5a48d550"
PIECES = {"飛": "R", "角": "B", "金": "G", "銀": "S", "桂": "N", "香": "L", "歩": "P", "玉": "K", "王": "K",
          "龍": "+R", "馬": "+B", "と": "+P", "成香": "+L", "成桂": "+N", "成銀": "+S"}
COUNT = {"一": 1, "二": 2, "三": 3, "四": 4, "五": 5, "六": 6, "七": 7, "八": 8, "九": 9}


class KifTag(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.values = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag == "shogi-time":
            value = dict(attrs).get("kif")
            if value is not None:
                self.values.append(value)


def hand(text: str, heading: str, lower: bool) -> str:
    value = re.search(rf"^{heading}の持駒：([^\n]+)$", text, re.M)
    assert value, heading
    found = {piece: int(COUNT.get(count, 1)) for piece, count in re.findall(r"([飛角金銀桂香歩])([一二三四五六七八九]?)", value.group(1))}
    assert found
    return "".join(f"{found.get(piece, 0) if found.get(piece, 0) > 1 else ''}{PIECES[piece].lower() if lower else PIECES[piece]}"
                   for piece in "飛角金銀桂香歩" if piece in found)


def main() -> None:
    raw = SOURCE.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == SOURCE_SHA
    article = raw.decode("utf-8")
    start = article.index('id="i-3">問題その２')
    end = article.index('id="i-4">問題その３', start)
    segment = article[start:end]
    parser = KifTag()
    parser.feed(segment)
    diagram = next(value for value in parser.values if "手数＝103" in value and "後手番" in value)
    rows = []
    for line in diagram.splitlines():
        if not line.startswith("|"):
            continue
        cells = re.findall(r"v?(?:成香|成桂|成銀|龍|馬|と|香|飛|歩|銀|金|角|桂|玉|王|・)", line)
        assert len(cells) == 9, (line, cells)
        blanks = 0
        row = ""
        for cell in cells:
            if cell == "・":
                blanks += 1
                continue
            if blanks:
                row += str(blanks)
                blanks = 0
            white = cell.startswith("v")
            name = cell[1:] if white else cell
            piece = PIECES[name]
            row += piece.lower() if white else piece
        if blanks:
            row += str(blanks)
        rows.append(row)
    assert len(rows) == 9
    holdings = hand(diagram, "先手", False) + hand(diagram, "後手", True)
    sfen = f"{'/'.join(rows)} w {holdings} 104"
    assert sfen.split()[0].count("K") == 1 and sfen.split()[0].count("k") == 1
    verify = subprocess.run([str(VERIFY), sfen, "R*7i"], capture_output=True, text=True, timeout=3, check=False)
    assert verify.returncode == 0 and "first_move_legal=true" in verify.stdout, verify.stderr + verify.stdout
    receipt = {
        "schema": "sekirei.pt71b-article.v1",
        "source_url": "https://suimon-shogi.com/tumeshogi-01/",
        "source_html_sha256": SOURCE_SHA,
        "game_id": "wdoor+floodgate-900-0+ponanza-990XEE+NineDayFever_XeonE5-2690_16c+20130531163001",
        "article_position_after_plies": 103,
        "article_label": "実戦は19手、最長21手",
        "diagram": diagram,
        "sfen": sfen,
        "next_recorded_move": "R*7i",
        "next_move_verification": verify.stdout.strip(),
        "verification_binary_sha256": hashlib.sha256(VERIFY.read_bytes()).hexdigest(),
        "official_csa_replayed": False,
        "official_csa_http_status": 404,
    }
    OUT.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    print(sfen)
    print(verify.stdout.strip())


if __name__ == "__main__":
    main()
