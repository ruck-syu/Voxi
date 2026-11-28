// Adds a command request to the append-only tracking issue queue.
const { commandRequests, createHandler, ipKey } = require("./_requests.js");

module.exports = createHandler(commandRequests);
module.exports.createHandler = (options) => createHandler(commandRequests, options);
module.exports.ipKey = ipKey;
