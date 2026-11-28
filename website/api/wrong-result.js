// Adds an opted-in wrong-result report to the append-only tracking issue queue.
const { wrongResults, createHandler } = require("./_requests.js");

module.exports = createHandler(wrongResults);
module.exports.createHandler = (options) => createHandler(wrongResults, options);
