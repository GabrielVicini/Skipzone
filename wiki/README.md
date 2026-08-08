# wiki/

Source for the Skipzone GitHub wiki. It lives in the main repository, not in the
`.wiki` repository, so that it is versioned with the code it describes and a
change that invalidates a page can be caught in the same commit.

Start at [Home.md](Home.md).

## Publishing it

GitHub wikis are a separate git repository at
`https://github.com/<owner>/<repo>.wiki.git`. To publish, clone it and copy
these files across:

```bash
git clone https://github.com/GabrielVicini/Skipzone.wiki.git /tmp/skipzone-wiki
cp wiki/*.md /tmp/skipzone-wiki/
cd /tmp/skipzone-wiki && git add -A && git commit -m "Sync wiki from main repo" && git push
```

The wiki must be enabled and initialised once through the repository settings
before that clone URL exists.

## Conventions for these pages

- **Flat namespace.** GitHub wikis have no directories. A page's title is its
  filename with hyphens turned into spaces, so `Command-Line-Tools.md` becomes
  "Command Line Tools".
- **`_Sidebar.md` and `_Footer.md`** are rendered on every page by GitHub. They
  do not appear as pages of their own.
- **Links are plain relative filenames including the `.md`.** That form works
  both on GitHub's wiki renderer and when reading the files directly in the
  repository, which the bare-title form does not.
- **This `README.md` is not part of the wiki.** Do not copy it across; GitHub
  would render it as a page called "README".
- **No em dashes.**
