import "react";

declare module "react" {
  // Merging requires the same type parameter as React's declaration.
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  interface HTMLAttributes<T> {
    /** WebKit's standard writing-suggestions opt-out; not yet in our React types. */
    writingsuggestions?: "true" | "false";
  }
}
