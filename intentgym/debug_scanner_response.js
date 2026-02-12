// Debug script to test scanner response structure
// Run in browser console after loading scanner.js

(async function() {
    // Simulate the scan command that the backend sends
    const command = { action: "scan" };

    console.log("Sending command:", JSON.stringify(command));

    // Call scanner's process function (same as backend does)
    const response = await window.Oryn.process(command);

    console.log("\n=== RESPONSE ===");
    console.log("Type:", typeof response);
    console.log("Status:", response.status);
    console.log("Response keys:", Object.keys(response));

    console.log("\n=== FULL RESPONSE (first 500 chars) ===");
    const jsonStr = JSON.stringify(response);
    console.log(jsonStr.substring(0, 500));

    console.log("\n=== HAS REQUIRED FIELDS? ===");
    console.log("Has 'page'?", 'page' in response);
    console.log("Has 'elements'?", 'elements' in response);
    console.log("Has 'stats'?", 'stats' in response);

    if (response.page) {
        console.log("\nPage structure:", Object.keys(response.page));
    }

    if (response.stats) {
        console.log("Stats structure:", Object.keys(response.stats));
    }

    return response;
})();
