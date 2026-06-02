pub mod chunker;
pub mod parser;
pub mod shell_ast;
pub mod symbols;

pub use chunker::{chunk_file, CodeChunk};
pub use parser::{CodeParser, ParsedTree};
pub use shell_ast::{
    extract_command_names, has_command_substitution, nesting_depth, parse_shell_ast, CaseArm,
    RedirectOp, Redirection, ShellArg, ShellAst,
};
pub use symbols::{extract_callees, extract_symbols, extract_trait_impls, Symbol, SymbolKind};
