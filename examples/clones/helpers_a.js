/**
 * Helper utilities - Module A
 */

function formatCurrency(amount, currencyCode) {
    if (typeof amount !== "number" || isNaN(amount)) {
        return "Invalid amount";
    }

    const formatted = Math.abs(amount).toFixed(2);
    const sign = amount < 0 ? "-" : "";

    switch (currencyCode) {
        case "USD":
            return sign + "$" + formatted;
        case "EUR":
            return sign + "€" + formatted;
        case "GBP":
            return sign + "£" + formatted;
        case "JPY":
            return sign + "¥" + Math.round(Math.abs(amount));
        default:
            return sign + formatted + " " + currencyCode;
    }
}

function validatePassword(password) {
    const errors = [];

    if (!password || typeof password !== "string") {
        return { valid: false, errors: ["Password is required"] };
    }

    if (password.length < 8) {
        errors.push("Password must be at least 8 characters");
    }

    if (password.length > 128) {
        errors.push("Password must be at most 128 characters");
    }

    if (!/[A-Z]/.test(password)) {
        errors.push("Password must contain at least one uppercase letter");
    }

    if (!/[a-z]/.test(password)) {
        errors.push("Password must contain at least one lowercase letter");
    }

    if (!/[0-9]/.test(password)) {
        errors.push("Password must contain at least one digit");
    }

    if (!/[!@#$%^&*()_+\-=\[\]{}|;:,.<>?]/.test(password)) {
        errors.push("Password must contain at least one special character");
    }

    return {
        valid: errors.length === 0,
        errors: errors,
    };
}

function truncateText(text, maxLength, suffix) {
    if (!text) return "";
    if (!suffix) suffix = "...";
    if (!maxLength) maxLength = 100;

    if (text.length <= maxLength) {
        return text;
    }

    const truncated = text.substring(0, maxLength - suffix.length);
    const lastSpace = truncated.lastIndexOf(" ");

    if (lastSpace > maxLength * 0.5) {
        return truncated.substring(0, lastSpace) + suffix;
    }

    return truncated + suffix;
}

function debounce(func, delay) {
    let timeoutId = null;

    return function(...args) {
        if (timeoutId) {
            clearTimeout(timeoutId);
        }

        timeoutId = setTimeout(() => {
            func.apply(this, args);
            timeoutId = null;
        }, delay);
    };
}
