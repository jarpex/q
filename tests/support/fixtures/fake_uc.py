"""Stand-in for the uc program in the q integration tests.

The real program cleans typographic garbage from the text of the model. This
one removes the non-breaking spaces, which is enough to tell whether the text
went through the program at all.

The program records the arguments it was called with in a file next to itself,
so the test can check what q passes to it.
"""

import os
import sys


def _args_file():
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "uc_args.txt")


def main():
    with open(_args_file(), "w", encoding="utf-8") as handle:
        handle.write("\n".join(sys.argv[1:]))

    text = sys.stdin.read()
    sys.stdout.write(text.replace("\u00a0", ""))


if __name__ == "__main__":
    main()
