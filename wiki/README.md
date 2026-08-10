# wiki/

Source for the Skipzone GitHub wiki. It lives in the main repository, not in the
`.wiki` repository, so that it is versioned with the code it describes and a
change that invalidates a page can be caught in the same commit.

Read it at <https://github.com/GabrielVicini/Skipzone/wiki>. The sources here
start at [Home.md](Home.md); note that the links *between* pages are wiki page
titles rather than filenames (see the conventions below), so they resolve on the
published wiki and not in this directory.

## Publishing it

GitHub wikis are a separate git repository at
`https://github.com/<owner>/<repo>.wiki.git`. To publish, clone it beside this
repository and copy the pages across. From the repository root:

```bash
git clone https://github.com/GabrielVicini/Skipzone.wiki.git ../Skipzone.wiki
```

```bash
find wiki -maxdepth 1 -name '*.md' ! -name 'README.md' -exec cp {} ../Skipzone.wiki/ \; && git -C ../Skipzone.wiki add -A && git -C ../Skipzone.wiki commit -m "Sync wiki from main repo" && git -C ../Skipzone.wiki push
```

The same thing in PowerShell, which is what Windows gets and which has no
`find`:

```powershell
Get-ChildItem wiki\*.md -Exclude README.md | Copy-Item -Destination ..\Skipzone.wiki
git -C ..\Skipzone.wiki add -A
git -C ..\Skipzone.wiki commit -m "Sync wiki from main repo"
git -C ..\Skipzone.wiki push
```

`README.md` is excluded deliberately - see the conventions below. The copy step
used to be a plain `cp wiki/*.md`, which contradicted that rule and published a
stray page called "README" describing how to publish the wiki. If that page is
still live, `git rm README.md` in the wiki clone once.

Copying only adds and overwrites. A page **deleted** here stays on the wiki until
it is deleted there too, so removing a page means `git rm` in the wiki clone as
well.

The wiki must be enabled and initialised once through the repository settings
(Settings > Features > Wikis, then create the first page) before that clone URL
exists at all; cloning it before then fails with a repository-not-found error.

## Conventions for these pages

- **Flat namespace.** GitHub wikis have no directories. A page's title is its
  filename with hyphens turned into spaces, so `Command-Line-Tools.md` becomes
  "Command Line Tools".
- **`_Sidebar.md` and `_Footer.md`** are rendered on every page by GitHub. They
  do not appear as pages of their own.
- **Links between pages are bare page titles, with no `.md`.** `[Architecture](Architecture)`,
  and `[Architecture](Architecture#some-heading)` for a section.

  This used to say the opposite - keep the `.md`, because that form also works
  when the files are read directly in the repository. It does not work where it
  matters. A wiki page is served at `/wiki/<Title>`, so a relative link to
  `Architecture.md` resolves to `/wiki/Architecture.md`, which GitHub serves as
  the **raw markdown source**: clicking a link in the sidebar dumped the file's
  text on screen instead of opening the page. The rendered wiki is the published
  artefact and it wins; reading the sources directly is the fallback, and there
  the links simply do not resolve, which is a far smaller cost than a wiki whose
  navigation does not work.
- **This `README.md` is not part of the wiki.** Do not copy it across; GitHub
  would render it as a page called "README".