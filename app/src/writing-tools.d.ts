import "react";

declare module "react" {
  interface HTMLAttributes<T> {
    /** WebKit's standard writing-suggestions opt-out; not yet in our React types. */
    writingsuggestions?: "true" | "false";
  }
}
