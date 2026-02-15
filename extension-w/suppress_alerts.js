// Override window.alert
window.alert = function(message) {
    console.log("Oryn suppressed alert:", message);
};

// Override window.confirm
window.confirm = function(message) {
    console.log("Oryn suppressed confirm:", message);
    return true; // Always accept
};

// Override window.prompt
window.prompt = function(message, defaultValue) {
    console.log("Oryn suppressed prompt:", message);
    return defaultValue || ""; // Return empty string or default
};
