"""Utility functions - version 1."""

def format_file_size(size_bytes):
    """Format a file size in bytes to a human-readable string."""
    if size_bytes < 0:
        raise ValueError("Size must be non-negative")

    units = ["B", "KB", "MB", "GB", "TB", "PB"]
    unit_index = 0
    size = float(size_bytes)

    while size >= 1024 and unit_index < len(units) - 1:
        size /= 1024.0
        unit_index += 1

    if unit_index == 0:
        return f"{int(size)} {units[unit_index]}"
    return f"{size:.2f} {units[unit_index]}"


def validate_email_address(email):
    """Validate an email address format."""
    if not email or not isinstance(email, str):
        return False

    parts = email.strip().split("@")
    if len(parts) != 2:
        return False

    local_part, domain = parts
    if not local_part or not domain:
        return False

    if "." not in domain:
        return False

    if len(local_part) > 64:
        return False

    if len(domain) > 253:
        return False

    return True


def sanitize_username(username):
    """Sanitize a username by removing invalid characters."""
    if not username:
        return ""

    # Strip whitespace
    cleaned = username.strip()

    # Only allow alphanumeric and underscore
    result = []
    for char in cleaned:
        if char.isalnum() or char == "_":
            result.append(char)

    sanitized = "".join(result)

    # Enforce length limits
    if len(sanitized) < 3:
        return ""
    if len(sanitized) > 30:
        sanitized = sanitized[:30]

    return sanitized.lower()


def calculate_pagination(total_items, page_size, current_page):
    """Calculate pagination metadata."""
    if page_size <= 0:
        page_size = 10
    if current_page < 1:
        current_page = 1

    total_pages = (total_items + page_size - 1) // page_size
    if current_page > total_pages:
        current_page = total_pages

    offset = (current_page - 1) * page_size
    has_prev = current_page > 1
    has_next = current_page < total_pages

    return {
        "total_items": total_items,
        "page_size": page_size,
        "current_page": current_page,
        "total_pages": total_pages,
        "offset": offset,
        "has_previous": has_prev,
        "has_next": has_next,
    }


def format_duration(seconds):
    """Format a duration in seconds to human-readable string."""
    if seconds < 0:
        return "0s"

    hours = int(seconds // 3600)
    minutes = int((seconds % 3600) // 60)
    secs = int(seconds % 60)

    parts = []
    if hours > 0:
        parts.append(f"{hours}h")
    if minutes > 0:
        parts.append(f"{minutes}m")
    if secs > 0 or not parts:
        parts.append(f"{secs}s")

    return " ".join(parts)
