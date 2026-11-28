// Adds an opted-in wrong-result report to the append-only tracking issue queue.
import { wrongResults, createHandler as createRequestHandler } from "./_requests.js";

export default createRequestHandler(wrongResults);
export const createHandler = (options) => createRequestHandler(wrongResults, options);
