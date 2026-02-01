"""Utility functions - version 2 (copied and slightly modified)."""

def format_file_size(num_bytes):
    """Format a file size in bytes to a human-readable string."""
    if num_bytes < 0:
        raise ValueError("Size cannot be negative")

    units = ["B", "KB", "MB", "GB", "TB", "PB"]
    idx = 0
    size = float(num_bytes)

    while size >= 1024 and idx < len(units) - 1:
        size /= 1024.0
        idx += 1

    if idx == 0:
        return f"{int(size)} {units[idx]}"
    return f"{size:.2f} {units[idx]}"


def validate_email_address(email_str):
    """Validate an email address format."""
    if not email_str or not isinstance(email_str, str):
        return False

    parts = email_str.strip().split("@")
    if len(parts) != 2:
        return False

    local_part, domain_part = parts
    if not local_part or not domain_part:
        return False

    if "." not in domain_part:
        return False

    if len(local_part) > 64:
        return False

    if len(domain_part) > 253:
        return False

    return True


def sanitize_username(raw_username):
    """Sanitize a username by removing invalid characters."""
    if not raw_username:
        return ""

    # Strip whitespace
    cleaned = raw_username.strip()

    # Only allow alphanumeric and underscore
    result = []
    for ch in cleaned:
        if ch.isalnum() or ch == "_":
            result.append(ch)

    sanitized = "".join(result)

    # Enforce length limits
    if len(sanitized) < 3:
        return ""
    if len(sanitized) > 30:
        sanitized = sanitized[:30]

    return sanitized.lower()


def calculate_pagination(total_count, items_per_page, page_num):
    """Calculate pagination metadata."""
    if items_per_page <= 0:
        items_per_page = 10
    if page_num < 1:
        page_num = 1

    num_pages = (total_count + items_per_page - 1) // items_per_page
    if page_num > num_pages:
        page_num = num_pages

    start_offset = (page_num - 1) * items_per_page
    has_prev = page_num > 1
    has_next = page_num < num_pages

    return {
        "total_items": total_count,
        "page_size": items_per_page,
        "current_page": page_num,
        "total_pages": num_pages,
        "offset": start_offset,
        "has_previous": has_prev,
        "has_next": has_next,
    }


def format_duration(total_seconds):
    """Format a duration in seconds to human-readable string."""
    if total_seconds < 0:
        return "0s"

    hours = int(total_seconds // 3600)
    minutes = int((total_seconds % 3600) // 60)
    secs = int(total_seconds % 60)

    parts = []
    if hours > 0:
        parts.append(f"{hours}h")
    if minutes > 0:
        parts.append(f"{minutes}m")
    if secs > 0 or not parts:
        parts.append(f"{secs}s")

    return " ".join(parts)
