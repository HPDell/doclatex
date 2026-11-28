# DocLaTeX

[English](README.md) | **简体中文**

DocLaTeX 是一个实验性的 Rust 命令行工具，可将面向学术写作的 LaTeX 源码直接转换为 Microsoft Word `.docx` 文档。它自行实现了词法分析器、宏展开器、文档解析器、参考文献处理器、数学公式到 OMML 的转换器以及 OOXML 写入器，**不会调用** TeX/LaTeX 引擎、BibTeX 或 Biber。

项目的主要目标是保留文档语义，而不是近似复刻 PDF 页面：生成可编辑的 Word 公式、可用的交叉引用、自动标题与公式编号、引文、术语表以及可复用的 Word 样式。

> **项目状态：** DocLaTeX 支持较丰富但并不完整的 LaTeX 子集。它不是完整的 TeX 实现，也不能作为所有宏包和文档类的直接替代品。转换时请检查警告，并对重要文档进行视觉核验。

## 已实现功能

### 宏展开与源码处理

- 面向 TeX 的惰性词法分析，支持 `\makeatletter`、`\makeatother` 等类别码（category code）变更。
- 支持 `\def`、`\gdef`、`\edef`、`\xdef`、`\let`、`\newcommand`、`\renewcommand`、`\providecommand` 和 `\DeclareRobustCommand` 定义宏。
- 通过 `\NewDocumentCommand` 等相关命令支持部分 `xparse` 参数格式，包括必选参数和可选参数。
- 在正文和数学公式中展开用户宏，并支持自定义数学算子。
- 支持条件判断、计数器、数字格式、自定义环境和定理声明。
- 支持递归处理 `\input` 和 `\include`。
- 支持 `filecontents` 环境，可按文档要求生成局部源码或资源文件。

### 文档类与宏包

- 解析 `\documentclass`、`\LoadClass`、`\usepackage` 和 `\RequirePackage`。
- 从本地 `.cls` 和 `.sty` 文件加载宏定义，包括由其他文档类或宏包继续加载的文件。
- 支持文档类/宏包选项，包括选项声明、处理、执行和转发。
- 相对于当前源码文件及项目目录查找文件，并支持通过 `TEXINPUTS` 指定以冒号分隔的附加搜索目录。
- 内置常见的 `article`、`report`、`book` 行为，支持部分字号选项、`a4paper` 选项和基础中文文档类/宏包行为。
- 识别 `amsmath`、`graphicx`、`hyperref`、`glossaries`、`natbib`、`cleveref`、`booktabs`、`geometry` 等常见学术写作宏包。

文档类与宏包支持均为语义上的部分支持：能够识别某个宏包名称，并不意味着已经实现该宏包提供的所有命令。

### 文档结构与文本

- 支持 part、chapter、section、subsection 及更低层级标题，包括星号标题和附录字母编号。
- 使用 Word 标题样式和多级编号列表实现标题编号。
- `\maketitle` 支持标题、作者、日期和 `\thanks` 内容。
- 使用 Word 域实现 `\tableofcontents` 目录。
- 支持段落对齐、缩进、手动换行、分页和基础垂直间距。
- 支持粗体、斜体、倾斜体、小型大写字母、下划线、删除线、等宽字体、文字颜色、高亮、上标和下标。
- 支持项目符号列表、编号列表、嵌套列表、自定义项目标签和描述列表。
- 支持脚注、外部超链接、 verbatim 文本和 `lstlisting` 内容。
- 支持 `abstract`、引文、对齐、`minipage`、定理类和用户自定义环境。
- 其他以段落形式呈现的环境会生成对应的 Word 段落样式；`abstract` 使用专门的 `Abstract` 样式。

### 数学公式

- 将行内和行间 LaTeX 数学公式直接转换为可编辑的 Office Math Markup Language（OMML），而不是插入公式截图。
- 支持分式、根式、上下标、大型运算符及其上下限、定界符、重音符号、上下花括号、矩阵、数组、cases、方框表达式、幻影内容和常见希腊字母/符号/算子命令。
- 支持 `\mathrm`、`\mathbf`、`\mathcal`、`\mathbb`、`\mathfrak`、`\text` 等数学字母和文本命令。
- 支持 `equation`、`align`、`gather`、`multline`、`alignat`、`eqnarray` 及对应的星号环境，并支持 `subequations` 和逐行 `\nonumber`/`\notag`。
- 在 `&` 对应位置使用 Word 手动对齐标记实现对齐。
- 使用 Word `SEQ` 域生成公式编号，并按照专业模式公式的 `#(编号)` 约定安排编号位置。
- 在公式编号处创建书签，同时保存引用的缓存文本，使域更新前也能显示内容。

### 交叉引用

- 支持标题、图片、表格、公式和定理类环境中的 `\label` 目标。
- 支持 `\ref`、`\eqref`、`\pageref`、`\nameref`、`\autoref`、`\cref` 和 `\Cref`。
- 使用 Word 书签以及 `REF`/`PAGEREF` 域，并创建指向引用目标的超链接。
- 支持通过 `\crefname` 和 `\Crefname` 自定义引用名称。
- 使用两遍编译流程和项目专用的 `.aux` 文件，在两遍之间传递标签与参考文献信息。

### 参考文献与引文

- 直接解析 `.bib` 文件，无需安装外部 BibTeX。
- 支持多个参考文献文件、一条命令中的多个引用键、引用排序/压缩行为以及 `\nocite{*}`。
- 支持 `\cite`、`\citet`、`\citep`、`\citealt`、`\citealp`、`\citeauthor`、`\citeyear`、`\citeyearpar` 和 `\citeonline`。
- 对识别到的 `plain`、`unsrt`、`alpha`、`abbrv`、`ieeetr`/`IEEEtran`、`apalike` 及相关样式，生成数字型、字母型、IEEE 风格或作者—年份型输出。
- 支持手写的 `thebibliography` 环境和 `\bibitem` 条目。
- 支持从引文跳转到对应参考文献条目的内部链接。

这里的参考文献格式化器比 BibTeX 的 `.bst` 语言更精简，无法复现所有期刊样式。

### 缩写与术语表

- 支持 `\newacronym` 和 `\newglossaryentry` 定义。
- 支持首次使用展开、后续使用简称、首字母大写、复数形式，以及显式的简称/全称/完整缩写命令。
- 支持常见 `\gls`、`\glspl`、`\acrshort`、`\acrlong`、`\acrfull` 及 acronym 宏包别名。
- 支持 `\printglossaries`、`\printglossary` 和 `\printacronyms`，无需运行 `makeindex` 或 `xindy`。

### 图片与表格

- 将 PNG、JPEG 和 SVG 图片直接嵌入 DOCX 包。
- 将 PDF 图片转换为 SVG 后嵌入；仅使用 PDF 的第一页。
- `\includegraphics` 支持 `width`、`height` 和 `scale`，包括相对于 `\textwidth` 的长度，并在适当情况下保持纵横比。
- 图片默认居中，使用专门的 `Picture`（图片）样式，并设置为与下段同页。
- 支持图片/表格题注和标签，分别使用内置 `Caption`（题注）与自定义 `TableCaption`（表格题注）样式。
- `tabular`、`tabularx` 和 `tabular*` 默认居中，支持列对齐、段落列宽、横线、边框和已实现的单元格跨行/跨列。
- 表格内容使用 `TableText`（表格正文）样式：单倍行距，无首行缩进或悬挂缩进。

### Word 文档生成

- 直接生成文档内容、样式、编号、关系、设置、属性、脚注和媒体等 OOXML 部件并打包为 DOCX。
- 在适当位置使用 Word 内置样式，包括正文文本、正文文本首行缩进、标题、列表、题注、脚注和参考文献。
- 为图片、表格正文、表格题注、摘要、逐字文本及其他已解析环境提供专门样式。
- Word 域会标记为自动更新，同时写入缓存值，使域更新前也能显示内容。

## 环境要求与构建

- 安装包含 Cargo 的 Rust 工具链。仓库目前未固定具体的 Rust 版本。
- 仅在处理 PDF 图片时需要：确保 `pdftocairo` 或 `mutool` 位于 `PATH` 中。程序会优先尝试 `pdftocairo -svg`，失败后尝试 `mutool draw`。

构建优化后的可执行文件：

```sh
cargo build --release
```

生成的可执行文件位于 `target/release/doclatex`。

## 使用方法

```text
doclatex <input.tex> [-o <output.docx>]
doclatex --help
doclatex --version
```

例如，在本仓库中直接运行：

```sh
cargo run -- path/to/main.tex --output path/to/main.docx
```

也可以运行发布模式构建的二进制文件：

```sh
target/release/doclatex path/to/main.tex -o path/to/main.docx
```

如果省略 `-o`/`--output`，会将输入文件的扩展名替换为 `.docx`。每次编译自动执行两遍展开/解析，并在输入文件旁写入 `.aux` 文件。已有输出文件和 `.aux` 文件会被覆盖。

相对输入和输出路径均相对于调用 DocLaTeX 时所在的目录解析。被包含的源码、文档类、宏包、图片和参考文献应保留在项目原有的相对路径上。

## 示例

`examples/demo.tex` 是宏、交叉引用、公式、缩写、表格、图片、逐字文本和参考文献的精简示例。以下命令可在不覆盖仓库内示例输出的情况下运行：

```sh
tmpdir=$(mktemp -d)
cp examples/demo.tex examples/refs.bib examples/pixel.png examples/vector.pdf "$tmpdir"/
cargo run -- "$tmpdir/demo.tex" --output "$tmpdir/result.docx"
```

`examples/attention-is-all-you-need/` 提供了基于 *Attention Is All You Need* TeX 源码的大型多文件示例，涵盖递归 `\input`、自定义 `.sty` 文件、数学公式、表格和本地图片。

## 打开并更新 DOCX 文档

生成的文档包含域的缓存结果，并请求编辑器在打开文档时更新域。根据 Word 兼容编辑器及其安全设置，仍可能需要手动更新域。在 Microsoft Word 中可选中文档内容并按 **F9**；如有提示，选择更新整个目录。

目录、页码引用、公式序列域以及后续编辑引起的编号变化尤其需要注意。Microsoft Word、WPS Office 和其他 OOXML 编辑器的排版可能不同，建议进行视觉检查。

## 尚未支持或仅部分支持的特性

- **完整 TeX 兼容性：** 不保证支持复杂的定界宏场景、完整 TeX 编程模型、所有类别码/原语行为以及任意第三方宏包。
- **完整复刻文档类/宏包：** 不执行完整 TeX 安装中的代码或任意 `.bst` 文件。页面几何、字体选择、页眉页脚、间距和文档类专有版式只能部分复现。
- **TeX 级别的分页：** 浮动体按源码位置输出，不使用 TeX 的浮动体放置算法。浮动位置参数可以被接受，但不会控制 Word 分页。断行、断字、孤行/寡行控制和页面优化主要交给编辑器处理。
- **基础位图/矢量图以外的图形：** `tikzpicture` 内容会被明确跳过。目前不渲染 PGFPlots、TeX 绘图代码、EPS 图片和任意图形环境。
- **列表与索引：** `\listoffigures` 和 `\listoftables` 会被跳过。尚未实现通用索引生成以及 `makeindex`/`xindy` 处理。
- **完整参考文献引擎：** 不支持 `biblatex`、Biber、CSL 样式和任意 BibTeX `.bst` 程序。复杂 BibTeX 字段格式可能与 TeX 输出不同。
- **完整术语表样式：** 术语条目由程序直接生成，不依赖外部索引工具；高级排序、层级和自定义术语表样式尚未完整实现。
- **所有数学结构：** OMML 转换器涵盖常见学术记法，但罕见符号、复杂对齐、自定义间距和宏包专有数学命令可能缺失或呈现不同。
- **高级表格与浮动体：** 尚未完整复现 `longtable`、复杂跨页表格、子浮动体布局以及全部 `array`/宏包扩展。
- **逐像素的视觉一致性：** 输出是保留语义的 Word 文档，而不是 LaTeX 所生成 PDF 页面的复制品。

不支持的命令通常会被原样保留或跳过并产生警告，以便继续转换。这种尽力而为的行为可能生成不完整的文档；用于正式文档时不应忽略警告。

## 开发

在仓库根目录运行单元测试和格式检查：

```sh
cargo test
cargo fmt --check
```

项目采用单一 Rust 二进制 crate。主要阶段包括 `token.rs`（词法分析）、`engine.rs`（宏展开及文档类/宏包加载）、`parse.rs`（文档语义）、`math.rs`（OMML）、`bib.rs`（参考文献）、`aux.rs`（两遍编译数据）和 `docx.rs`（OOXML/DOCX 输出）。

仓库中已提交的 `.docx` 示例仅用于展示，并不是测试套件进行比较的黄金文件。

## 安全提示

转换过程不在沙箱中运行。LaTeX 输入可以通过文件包含、文档类、宏包、图片和参考文献读取本地文件，`filecontents` 也可以写入文件。编译不可信文档前请先审查其内容，测试时建议使用临时项目副本。
