pub mod parser;
pub mod symbols;
pub mod chunker;
pub mod shell_ast;

pub use parser::{CodeParser, ParsedTree};
pub use symbols::{Symbol, SymbolKind, extract_symbols};
pub use chunker::{CodeChunk, chunk_file};
pub use shell_ast::{
    ShellAst, ShellArg, CaseArm, Redirection, RedirectOp,
    parse_shell_ast, extract_command_names, has_command_substitution, nesting_depth,
};
