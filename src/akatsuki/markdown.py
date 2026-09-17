"""Markdown parsing, heading extraction, section slicing, and token budget packing."""

import re


def extract_headings(content: str) -> list[tuple[int, str, int]]:
    """Parse markdown content and return list of (level, heading_title, line_number)."""
    headings = []
    lines = content.splitlines()
    in_frontmatter = content.startswith("---")
    fm_dashes = 0
    in_code_fence = False

    for idx, line in enumerate(lines, 1):
        stripped = line.strip()
        if in_frontmatter:
            if stripped == "---":
                fm_dashes += 1
                if fm_dashes == 2:
                    in_frontmatter = False
            continue

        if stripped.startswith("```") or stripped.startswith("~~~"):
            in_code_fence = not in_code_fence
            continue

        if in_code_fence:
            continue

        m = re.match(r"^(#{1,6})\s+(.+)$", line)
        if m:
            level = len(m.group(1))
            heading_title = m.group(2).strip()
            headings.append((level, heading_title, idx))
    return headings


def normalize_heading(h: str) -> str:
    """Normalize heading for resilient matching by removing leading emojis and symbols."""
    return re.sub(r"^[^\w\s]+", "", h).strip().lower()


def slice_markdown_section(content: str, target: str) -> tuple[str | None, list[str]]:
    """Extract a markdown section matching target heading."""
    headings = extract_headings(content)
    toc_lines = [f"L{line_no}: {'#' * lvl} {title}" for lvl, title, line_no in headings]
    if not target or target.strip() == "__toc__":
        return None, toc_lines

    lines = content.splitlines()
    target_clean = normalize_heading(target)

    matched_idx = -1
    for i, (_lvl, title, _line_no) in enumerate(headings):
        if target_clean == normalize_heading(title) or target.lower() == title.lower():
            matched_idx = i
            break

    if matched_idx == -1:
        for i, (_lvl, title, _line_no) in enumerate(headings):
            if target_clean in normalize_heading(title) or target.lower() in title.lower():
                matched_idx = i
                break

    if matched_idx == -1:
        return None, toc_lines

    match_lvl, _match_title, match_line_no = headings[matched_idx]
    start_line_idx = match_line_no - 1

    end_line_idx = len(lines)
    for next_lvl, _next_title, next_line_no in headings[matched_idx + 1 :]:
        if next_lvl <= match_lvl:
            end_line_idx = next_line_no - 1
            break

    sliced_text = "\n".join(lines[start_line_idx:end_line_idx]).strip()
    return sliced_text, toc_lines


def replace_markdown_section(content: str, target: str, new_content: str) -> tuple[str, bool]:
    """Surgically replace the content of a markdown section matching target heading.
    Returns (new_full_content, success).
    """
    headings = extract_headings(content)
    if not headings:
        return content, False

    lines = content.splitlines(keepends=True)
    target_clean = normalize_heading(target)

    matched_idx = -1
    for i, (_lvl, title, _line_no) in enumerate(headings):
        if target_clean == normalize_heading(title) or target.lower() == title.lower():
            matched_idx = i
            break

    if matched_idx == -1:
        for i, (_lvl, title, _line_no) in enumerate(headings):
            if target_clean in normalize_heading(title) or target.lower() in title.lower():
                matched_idx = i
                break

    if matched_idx == -1:
        return content, False

    match_lvl, _match_title, match_line_no = headings[matched_idx]
    start_line_idx = match_line_no  # line directly after the heading

    end_line_idx = len(lines)
    for next_lvl, _next_title, next_line_no in headings[matched_idx + 1 :]:
        if next_lvl <= match_lvl:
            end_line_idx = next_line_no - 1
            break

    cleaned_new = new_content.strip()
    if cleaned_new.startswith("#"):
        # Replacement content includes its own heading; replace from match_line_no - 1
        replacement_lines = [cleaned_new + "\n\n"]
        new_lines = lines[: match_line_no - 1] + replacement_lines + lines[end_line_idx:]
    else:
        # Keep heading line, replace section body
        replacement_lines = ["\n" + cleaned_new + "\n\n"]
        new_lines = lines[:start_line_idx] + replacement_lines + lines[end_line_idx:]

    return "".join(new_lines), True


def apply_token_budget(text: str, budget: int | str | None) -> str:
    """Apply token budget packing to markdown text (heuristic ~4 chars/token)."""
    if budget is not None:
        try:
            budget = int(budget)
        except (ValueError, TypeError):
            budget = None
    if not budget or budget <= 0:
        return text
    char_budget = budget * 4
    if len(text) <= char_budget:
        return text

    lines = text.splitlines(keepends=True)
    packed = []
    current_chars = 0
    in_fm = text.startswith("---")
    fm_count = 0

    for line in lines:
        if in_fm:
            packed.append(line)
            current_chars += len(line)
            if line.strip() == "---":
                fm_count += 1
                if fm_count == 2:
                    in_fm = False
            continue

        if current_chars + len(line) > char_budget - 120:
            packed.append(f"\n[Notice: Output truncated to fit budget of ~{budget} tokens]\n")
            break
        packed.append(line)
        current_chars += len(line)

    return "".join(packed)
