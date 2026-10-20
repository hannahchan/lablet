Do the following file work in the directory you were given, one step at a time, and wait for each result before the next step. Use only the tools you were given. Work with files alone: print nothing about the machine, its user, its environment variables or its network.

1. Write `plan.md`: ten lines, numbered 1 to 10, each a short task a small team might do in a week. Make each line between forty and sixty characters.
2. Read `plan.md` whole.
3. Read lines 3 to 5 of `plan.md` only, using `offset` and `limit`.
4. Read the last two lines of `plan.md` only, using `offset` and `limit`.
5. Run `wc -l plan.md` and `wc -c plan.md` as one command.
6. Write `plan.md` again with the same ten lines and an eleventh line that says how many bytes the file had before the change, as the command reported it.
7. Read the first line of `plan.md` only.
8. Write `done.md` holding two lines: the first says `done`, the second repeats the eleventh line of `plan.md`.
9. Run `ls` so the listing shows both files.
10. Finish with a short text that says which files are in the directory and how many lines each has. Call no tool after that.
