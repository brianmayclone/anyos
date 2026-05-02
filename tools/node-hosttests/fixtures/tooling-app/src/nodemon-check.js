const defaults = require("nodemon/lib/config/defaults");
const parse = require("nodemon/lib/cli/parse");
const parsed = parse("node nodemon --watch src --ext js,json app.js");

console.log(defaults.restartable + ":" + parsed.script + ":" + parsed.watch.join(",") + ":" + parsed.ext);
