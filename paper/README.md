# qbix paper

- `paper.md`: English manuscript
- `paper-ja.md`: Japanese manuscript
- `paper.bib`: references
- `data/` and `scripts/`: benchmark figure data and generator
- `figures/qbi-index-structure.dot`: Graphviz source for Figure 1; the Pages
  workflow renders it to PNG before building the PDF
- `notes/`: working notes

GitHub Actions builds neutral preprint PDFs with Inara and LaTeX. The Japanese
PDF uses LuaLaTeX with `luatexja-fontspec` and Noto CJK fonts.
