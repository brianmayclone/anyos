const pkg = require("eslint/package.json");
const js = require("@eslint/js");

console.log(pkg.name + ":" + pkg.version + ":" + Object.keys(js.configs).sort().join(","));
