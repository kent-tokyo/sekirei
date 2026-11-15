"""Extract preregistered 25+ ply positions from saved public HTML."""

import hashlib
import html
import json
from pathlib import Path
import re


BASE = Path(__file__).parent
SOURCES = (
    (2, 15, 25, "jlyg0bwfn2"),
    (3, 9, 29, "lgoldo4sxn"),
    (3, 18, 25, "49smnkg_p3"),
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def listing(page: int) -> tuple[dict, str]:
    raw = Path(f"/private/tmp/sekirei-tsume-page{page}.html").read_bytes()
    blocks = [
        part.split("</script>", 1)[0]
        for part in raw.decode().split('<script type="application/ld+json">')[1:]
    ]
    itemlist = next(x for x in map(json.loads, blocks) if x.get("@type") == "ItemList")
    assert itemlist["numberOfItems"] == 24
    return itemlist, sha256(raw)


def main() -> None:
    lists = {page: listing(page) for page in (2, 3)}
    cases = []
    for n, (page, index, ply, slug) in enumerate(SOURCES, 1):
        item = lists[page][0]["itemListElement"][index - 1]
        url = f"https://tsumeshogi.com/problems/{slug}"
        title = item["item"]["headline"]
        assert item["position"] == index and item["item"]["url"] == url
        assert re.search(rf"(?<!\d){ply}手詰", title)
        raw = Path(f"/private/tmp/sekirei-tsume-pt67-{n}.html").read_bytes()
        source = raw.decode()
        sfen = html.unescape(re.search(r'data-sfen="([^"]+)"', source).group(1))
        solution = html.unescape(re.search(r'data-solution="([^"]*)"', source).group(1))
        board = sfen.split()[0]
        assert (board.count("K"), board.count("k")) == (1, 1)
        cases.append(
            dict(
                page=page,
                list_index=index,
                title=title,
                listed_ply=ply,
                source_url=url,
                html_sha256=sha256(raw),
                sfen=sfen,
                html_solution_attr=solution,
                black_kings=1,
                white_kings=1,
            )
        )
    out = dict(
        schema="sekirei.public-tsume-pt67-25plus.v1",
        listing_sha256={str(page): lists[page][1] for page in (2, 3)},
        cases=cases,
    )
    path = BASE / "corpus-pt67.json"
    path.write_text(json.dumps(out, ensure_ascii=False, indent=2) + "\n")
    print(path, sha256(path.read_bytes()), len(cases))


if __name__ == "__main__":
    main()
