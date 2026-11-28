// Adds a command request to the append-only tracking issue queue.
import { commandRequests, createHandler as createRequestHandler } from "./_requests.js";

export default createRequestHandler(commandRequests);
export const createHandler = (options) => createRequestHandler(commandRequests, options);
export { ipKey } from "./_requests.js";
