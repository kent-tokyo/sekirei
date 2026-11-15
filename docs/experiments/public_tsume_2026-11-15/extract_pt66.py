"""Extract the preregistered PT66 positions from saved source HTML."""

import hashlib
import html
import json
from pathlib import Path
import re


BASE = Path(__file__).parent
LIST_HTML = Path("/private/tmp/sekirei-tsume-page3.html")
CASES = ((6, 17, "pgt1mj6ikq"), (13, 19, "asipdynfmk"), (14, 17, "migumgysm0"))


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> None:
    list_bytes = LIST_HTML.read_bytes()
    blocks = [
        part.split("</script>", 1)[0]
        for part in list_bytes.decode().split('<script type="application/ld+json">')[1:]
    ]
    listing = next(x for x in map(json.loads, blocks) if x.get("@type") == "ItemList")
    assert listing["numberOfItems"] == 24
    cases = []
    for n, (index, ply, slug) in enumerate(CASES, 1):
        item = listing["itemListElement"][index - 1]
        url = f"https://tsumeshogi.com/problems/{slug}"
        title = item["item"]["headline"]
        assert item["position"] == index and item["item"]["url"] == url
        assert re.search(rf"(?<!\d){ply}手詰", title)
        raw = Path(f"/private/tmp/sekirei-tsume-pt66-{n}.html").read_bytes()
        source = raw.decode()
        sfen = html.unescape(re.search(r'data-sfen="([^"]+)"', source).group(1))
        solution = html.unescape(re.search(r'data-solution="([^"]*)"', source).group(1))
        board = sfen.split()[0]
        black, white = board.count("K"), board.count("k")
        assert (black, white) == (1, 1)
        cases.append(
            dict(
                list_index=index,
                title=title,
                listed_ply=ply,
                source_url=url,
                html_sha256=sha256(raw),
                sfen=sfen,
                html_solution_attr=solution,
                black_kings=black,
                white_kings=white,
            )
        )
    out = dict(
        schema="sekirei.public-tsume-pt66-sougyoku-positions.v1",
        list_url="https://tsumeshogi.com/tags/%E5%8F%8C%E7%8E%89?page=3",
        list_html_sha256=sha256(list_bytes),
        cases=cases,
    )
    path = BASE / "corpus-pt66.json"
    path.write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n")
    print(path, sha256(path.read_bytes()), len(cases))


if __name__ == "__main__":
    main()
