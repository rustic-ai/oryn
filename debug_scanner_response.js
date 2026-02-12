// Quick diagnostic script to check scanner.js response structure
// Run in browser console after loading a page

(async function() {
    const response = await window.Oryn.process({
        cmd: 'scan',
        max_elements: 200,
        include_hidden: false
    });

    console.log("Scanner Response:", JSON.stringify(response, null, 2));
    console.log("\nTop-level keys:", Object.keys(response));
    console.log("\nHas required fields for ScanResult:");
    console.log("  - status:", response.status);
    console.log("  - page:", !!response.page);
    console.log("  - elements:", Array.isArray(response.elements));
    console.log("  - stats:", !!response.stats);

    if (response.page) {
        console.log("\nPageInfo keys:", Object.keys(response.page));
    }
    if (response.elements && response.elements[0]) {
        console.log("\nFirst Element keys:", Object.keys(response.elements[0]));
    }
    if (response.stats) {
        console.log("\nScanStats keys:", Object.keys(response.stats));
    }
})();
