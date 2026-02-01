/**
 * Helper utilities - Module B (near-duplicate of Module A)
 */

function formatCurrency(value, currency) {
    if (typeof value !== "number" || isNaN(value)) {
        return "Invalid amount";
    }

    const formatted = Math.abs(value).toFixed(2);
    const sign = value < 0 ? "-" : "";

    switch (currency) {
        case "USD":
            return sign + "$" + formatted;
        case "EUR":
            return sign + "€" + formatted;
        case "GBP":
            return sign + "£" + formatted;
        case "JPY":
            return sign + "¥" + Math.round(Math.abs(value));
        default:
            return sign + formatted + " " + currency;
    }
}

function validatePassword(pwd) {
    const errors = [];

    if (!pwd || typeof pwd !== "string") {
        return { valid: false, errors: ["Password is required"] };
    }

    if (pwd.length < 8) {
        errors.push("Password must be at least 8 characters");
    }

    if (pwd.length > 128) {
        errors.push("Password must be at most 128 characters");
    }

    if (!/[A-Z]/.test(pwd)) {
        errors.push("Password must contain at least one uppercase letter");
    }

    if (!/[a-z]/.test(pwd)) {
        errors.push("Password must contain at least one lowercase letter");
    }

    if (!/[0-9]/.test(pwd)) {
        errors.push("Password must contain at least one digit");
    }

    if (!/[!@#$%^&*()_+\-=\[\]{}|;:,.<>?]/.test(pwd)) {
        errors.push("Password must contain at least one special character");
    }

    return {
        valid: errors.length === 0,
        errors: errors,
    };
}

function truncateText(str, limit, ellipsis) {
    if (!str) return "";
    if (!ellipsis) ellipsis = "...";
    if (!limit) limit = 100;

    if (str.length <= limit) {
        return str;
    }

    const truncated = str.substring(0, limit - ellipsis.length);
    const lastSpace = truncated.lastIndexOf(" ");

    if (lastSpace > limit * 0.5) {
        return truncated.substring(0, lastSpace) + ellipsis;
    }

    return truncated + ellipsis;
}

function debounce(callback, wait) {
    let timerId = null;

    return function(...args) {
        if (timerId) {
            clearTimeout(timerId);
        }

        timerId = setTimeout(() => {
            callback.apply(this, args);
            timerId = null;
        }, wait);
    };
}
