import sys

import cowsay


def main() -> None:
    cowsay.cow("hello from py_runnable, args=%r" % (sys.argv[1:],))


if __name__ == "__main__":
    main()
