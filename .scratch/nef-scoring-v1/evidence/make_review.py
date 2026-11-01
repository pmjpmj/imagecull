#!/usr/bin/env python3
"""Turn a cull CSV plus its previews into a review sheet.

Usage:
  make_review.py SCORES.csv PREVIEWS_DIR OUT.html OUT.md

The sheet exists so a photographer can judge the rubric without opening a RAW:
every frame's preview, the six answers the model gave, and a blank column for
their own star rating. It is evidence for ticket 08 (real-shoot validation),
not part of the product, so it lives outside the crate.
"""

import base64
import csv
import html
import sys
from pathlib import Path

SCORES = ["overall", "technical", "composition", "impact"]


def load(csv_path):
    with open(csv_path, newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def number(row, key):
    try:
        return float(row[key])
    except (KeyError, TypeError, ValueError):
        return None


def stars(row):
    try:
        return int(row["stars"])
    except (KeyError, TypeError, ValueError):
        return None


def sort_key(row):
    overall = number(row, "overall")
    return (overall is None, -(overall or 0), row["file"])


def preview_for(row, previews):
    stem = Path(row["file"]).stem
    candidate = previews / f"{stem}.jpg"
    return candidate if candidate.exists() else None


def data_uri(path):
    return "data:image/jpeg;base64," + base64.b64encode(path.read_bytes()).decode()


def cell(row, key):
    value = row.get(key, "")
    return value if value else "—"


def markdown(rows, previews, title):
    lines = [f"# {title}", ""]
    lines.append(
        "| frame | overall | technical | composition | impact | stars | verdict | "
        "conf (overall / stars) | status | your stars | note |"
    )
    lines.append("|---|---|---|---|---|---|---|---|---|---|---|")
    for row in rows:
        lines.append(
            "| {file} | {overall} | {technical} | {composition} | {impact} | {stars} | "
            "{verdict} | {co} / {cs} | {status} |  |  |".format(
                file=row["file"],
                overall=cell(row, "overall"),
                technical=cell(row, "technical"),
                composition=cell(row, "composition"),
                impact=cell(row, "impact"),
                stars=cell(row, "stars"),
                verdict=cell(row, "verdict"),
                co=cell(row, "conf_overall"),
                cs=cell(row, "conf_stars"),
                status=cell(row, "status"),
            )
        )
    lines.append("")
    lines.append(
        "Preview JPEGs come from `imagecull score --dry-run`: they are the exact "
        "bytes the model saw, upright and downscaled to the configured cap."
    )
    return "\n".join(lines) + "\n"


def document(rows, previews, title):
    parts = [
        "<!doctype html>",
        "<meta charset='utf-8'>",
        f"<title>{html.escape(title)}</title>",
        "<style>",
        "body{font:14px/1.45 -apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;margin:24px;background:#111;color:#eee}",
        "h1{font-size:20px;margin:0 0 4px}",
        "p.hint{color:#999;margin:0 0 24px}",
        ".grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(520px,1fr));gap:20px}",
        ".card{background:#1b1b1b;border-radius:8px;overflow:hidden;display:flex;flex-direction:column}",
        ".card img{width:100%;height:auto;display:block;background:#000}",
        ".meta{padding:12px 14px}",
        ".name{font-weight:600;margin-bottom:6px;word-break:break-all}",
        ".scores{display:grid;grid-template-columns:repeat(4,1fr);gap:6px;margin:8px 0}",
        ".score{background:#242424;border-radius:6px;padding:6px 8px}",
        ".score b{display:block;font-size:11px;color:#999;font-weight:500;text-transform:uppercase;letter-spacing:.04em}",
        ".score span{font-size:18px;font-variant-numeric:tabular-nums}",
        ".line{color:#bbb;font-size:13px;margin-top:6px}",
        ".stars{color:#f5c451;letter-spacing:2px}",
        ".fail{color:#f08a8a}",
        "</style>",
        f"<h1>{html.escape(title)}</h1>",
        "<p class='hint'>Model's answers, sorted by overall. Judge the frame yourself, then compare — "
        "write your rating next to the model's in the notes column of the markdown sheet.</p>",
        "<div class='grid'>",
    ]
    for row in rows:
        preview = preview_for(row, previews)
        image = (
            f"<img src='{data_uri(preview)}' alt='{html.escape(row['file'])}'>"
            if preview
            else "<div class='meta fail'>no preview written for this frame</div>"
        )
        star_count = stars(row)
        star_text = "★" * star_count + "☆" * (5 - star_count) if star_count else "—"
        status = row.get("status", "")
        status_line = (
            f"<div class='line'>{html.escape(status)}"
            + (f" — {html.escape(row.get('error', ''))}" if row.get("error") else "")
            + "</div>"
            if status and status != "ok"
            else ""
        )
        parts.append(
            "<div class='card'>"
            + image
            + "<div class='meta'>"
            + f"<div class='name'>{html.escape(row['file'])}</div>"
            + "<div class='scores'>"
            + "".join(
                f"<div class='score'><b>{key}</b><span>{cell(row, key)}</span></div>"
                for key in SCORES
            )
            + "</div>"
            + f"<div class='line'><span class='stars'>{star_text}</span> &nbsp; "
            + f"{cell(row, 'verdict')} &nbsp; conf {cell(row, 'conf_overall')} / {cell(row, 'conf_stars')}"
            + f" &nbsp; {cell(row, 'latency_ms')} ms</div>"
            + status_line
            + "</div></div>"
        )
    parts += ["</div>"]
    return "\n".join(parts) + "\n"


def main(argv):
    if len(argv) != 5:
        print(__doc__)
        return 2
    csv_path, previews_dir, html_path, md_path = argv[1:]
    rows = sorted(load(csv_path), key=sort_key)
    previews = Path(previews_dir)
    title = f"{csv_path} — {len(rows)} frames"
    Path(html_path).write_text(document(rows, previews, title), encoding="utf-8")
    Path(md_path).write_text(markdown(rows, previews, title), encoding="utf-8")
    print(f"{len(rows)} frames -> {html_path}, {md_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
