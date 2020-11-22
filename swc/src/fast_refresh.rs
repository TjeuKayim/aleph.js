// Copyright 2017-2020 The swc Project Developers. All rights reserved. MIT license.
// Copyright 2020 the Aleph.js authors. All rights reserved. MIT license.

// @ref https://github.com/facebook/react/blob/master/packages/react-refresh/src/ReactFreshBabelPlugin.js

use std::rc::Rc;
use swc_common::SourceMap;
use swc_common::{Spanned, DUMMY_SP};
use swc_ecma_ast::*;
use swc_ecma_utils::{private_ident, quote_ident};
use swc_ecma_visit::{noop_fold_type, Fold};

pub fn fast_refresh_fold(refresh_reg: &str, refresh_sig: &str, source: Rc<SourceMap>) -> impl Fold {
  FastRefreshFold {
    refresh_reg: refresh_reg.into(),
    refresh_sig: refresh_sig.into(),
    signature_index: 0,
    source,
  }
}

pub struct FastRefreshFold {
  refresh_reg: String,
  refresh_sig: String,
  signature_index: u32,
  source: Rc<SourceMap>,
}

#[derive(Clone, Debug)]
struct HookCall {
  key: String,
  is_builtin: bool,
}

#[derive(Clone, Debug)]
struct Signature {
  parent_ident: Ident,
  handle_ident: Ident,
  hook_calls: Vec<HookCall>,
}

impl FastRefreshFold {
  fn get_persistent_fc(
    &mut self,
    ident: &Ident,
    block_stmt: &mut BlockStmt,
  ) -> (Option<Ident>, Option<Signature>) {
    let fc_id = if is_componentish_name(ident.as_ref()) {
      Some(ident.clone())
    } else {
      None
    };
    let mut hook_calls = Vec::<HookCall>::new();
    let stmts = &block_stmt.stmts;
    stmts.into_iter().for_each(|stmt| match stmt {
      Stmt::Expr(ExprStmt { expr, .. }) => match &**expr {
        Expr::Call(call) => match self.get_hook_call(None, call) {
          Some(hc) => hook_calls.push(hc),
          _ => {}
        },
        _ => {}
      },
      Stmt::Decl(Decl::Var(var_decl)) => match var_decl.decls.as_slice() {
        [VarDeclarator {
          name,
          init: Some(init_expr),
          ..
        }] => match init_expr.as_ref() {
          Expr::Call(call) => match self.get_hook_call(Some(name.clone()), call) {
            Some(hc) => hook_calls.push(hc),
            _ => {}
          },
          _ => {}
        },
        _ => {}
      },
      _ => {}
    });
    let signature = if hook_calls.len() > 0 {
      let mut handle_ident = String::from("_s");
      self.signature_index = self.signature_index + 1;
      if self.signature_index > 1 {
        handle_ident.push_str(self.signature_index.to_string().as_str());
      };
      let handle_ident = private_ident!(handle_ident.as_str());
      block_stmt.stmts.insert(
        0,
        Stmt::Expr(ExprStmt {
          span: DUMMY_SP,
          expr: Box::new(Expr::Call(CallExpr {
            span: DUMMY_SP,
            callee: ExprOrSuper::Expr(Box::new(Expr::Ident(handle_ident.clone()))),
            args: vec![],
            type_args: None,
          })),
        }),
      );
      Some(Signature {
        parent_ident: ident.clone(),
        handle_ident,
        hook_calls,
      })
    } else {
      None
    };
    (fc_id, signature)
  }

  fn get_persistent_fc_from_var_decl(
    &mut self,
    var_decl: &mut VarDecl,
  ) -> (Option<Ident>, Option<Signature>) {
    match var_decl.decls.as_mut_slice() {
      [VarDeclarator {
        name: Pat::Ident(ident),
        init: Some(init_expr),
        ..
      }] => match init_expr.as_mut() {
        Expr::Fn(FnExpr {
          function: Function {
            body: Some(body), ..
          },
          ..
        }) => self.get_persistent_fc(ident, body),
        Expr::Arrow(ArrowExpr {
          body: BlockStmtOrExpr::BlockStmt(body),
          ..
        }) => self.get_persistent_fc(ident, body),
        _ => (None, None),
      },
      _ => (None, None),
    }
  }

  fn get_hook_call(&self, pat: Option<Pat>, call: &CallExpr) -> Option<HookCall> {
    let callee = match &call.callee {
      ExprOrSuper::Super(_) => return None,
      ExprOrSuper::Expr(callee) => &**callee,
    };

    let callee = match callee {
      // useState()
      Expr::Ident(id) => Some((None, id)),
      // React.useState()
      Expr::Member(expr) => match &expr.obj {
        ExprOrSuper::Expr(obj) => match &**obj {
          Expr::Ident(obj) => match &*expr.prop {
            Expr::Ident(prop) => Some((Some(obj.clone()), prop)),
            _ => None,
          },
          _ => None,
        },
        _ => None,
      },
      _ => None,
    };

    if let Some((obj, id)) = callee {
      let id = id.sym.as_ref();
      let is_builtin = is_builtin_hook(obj, id);
      if is_builtin
        || (id.len() > 3 && id.starts_with("use") && id[3..].starts_with(char::is_uppercase))
      {
        let mut key = id.to_owned();
        match pat {
          Some(pat) => {
            let name = self.source.span_to_snippet(pat.span()).unwrap();
            key.push('{');
            key.push_str(name.as_str());
            // `useState` first argument is initial state.
            if call.args.len() > 0 && is_builtin && id == "useState" {
              key.push('(');
              key.push_str(
                self
                  .source
                  .span_to_snippet(call.args[0].span())
                  .unwrap()
                  .as_str(),
              );
              key.push(')');
            }
            // `useReducer` second argument is initial state.
            if call.args.len() > 1 && is_builtin && id == "useReducer" {
              key.push('(');
              key.push_str(
                self
                  .source
                  .span_to_snippet(call.args[1].span())
                  .unwrap()
                  .as_str(),
              );
              key.push(')');
            }
            key.push('}');
          }
          _ => key.push_str("{}"),
        };
        return Some(HookCall { key, is_builtin });
      }
    }
    None
  }
}

impl Fold for FastRefreshFold {
  noop_fold_type!();

  fn fold_module_items(&mut self, module_items: Vec<ModuleItem>) -> Vec<ModuleItem> {
    let mut items = Vec::<ModuleItem>::new();
    let mut raw_items = Vec::<ModuleItem>::new();
    let mut registrations = Vec::<(Ident, Ident)>::new();
    let mut signatures = Vec::<Signature>::new();

    for mut item in module_items {
      let (fc_id, signature) = match &mut item {
        // function App() {}
        ModuleItem::Stmt(Stmt::Decl(Decl::Fn(FnDecl {
          ident,
          function: Function {
            body: Some(body), ..
          },
          ..
        }))) => self.get_persistent_fc(ident, body),

        // export function App() {}
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(ExportDecl {
          decl:
            Decl::Fn(FnDecl {
              ident,
              function: Function {
                body: Some(body), ..
              },
              ..
            }),
          ..
        })) => self.get_persistent_fc(ident, body),

        // export default function App() {}
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(ExportDefaultDecl {
          decl:
            DefaultDecl::Fn(FnExpr {
              ident: Some(ident),
              function: Function {
                body: Some(body), ..
              },
              ..
            }),
          ..
        })) => self.get_persistent_fc(ident, body),

        // const Foo = () => {}
        // export const App = () => {}
        ModuleItem::Stmt(Stmt::Decl(Decl::Var(var_decl)))
        | ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(ExportDecl {
          decl: Decl::Var(var_decl),
          ..
        })) => self.get_persistent_fc_from_var_decl(var_decl),

        _ => (None, None),
      };

      raw_items.push(item);

      if let Some(fc_id) = fc_id {
        let mut registration_handle = String::from("_c");
        let registration_index = registrations.len() + 1;
        if registration_index > 1 {
          registration_handle.push_str(&registration_index.to_string());
        };
        let registration_id = private_ident!(registration_handle.as_str());

        registrations.push((registration_id.clone(), fc_id.clone()));

        // _c = App;
        // _c2 = Foo;
        raw_items.push(ModuleItem::Stmt(Stmt::Expr(ExprStmt {
          span: DUMMY_SP,
          expr: Box::new(Expr::Assign(AssignExpr {
            span: DUMMY_SP,
            op: AssignOp::Assign,
            left: PatOrExpr::Pat(Box::new(Pat::Ident(registration_id))),
            right: Box::new(Expr::Ident(fc_id)),
          })),
        })));
      }

      if let Some(signature) = signature {
        signatures.push(signature);
      }
    }

    // var _c, _c2;
    if registrations.len() > 0 {
      items.push(ModuleItem::Stmt(Stmt::Decl(Decl::Var(VarDecl {
        span: DUMMY_SP,
        kind: VarDeclKind::Var,
        declare: false,
        decls: registrations
          .clone()
          .into_iter()
          .map(|registration| VarDeclarator {
            span: DUMMY_SP,
            name: Pat::Ident(registration.0),
            init: None,
            definite: false,
          })
          .collect(),
      }))));
    }

    // var _s = $RefreshSig$(), _s2 = $RefreshSig$();
    if signatures.len() > 0 {
      items.push(ModuleItem::Stmt(Stmt::Decl(Decl::Var(VarDecl {
        span: DUMMY_SP,
        kind: VarDeclKind::Var,
        declare: false,
        decls: signatures
          .clone()
          .into_iter()
          .map(|signature| VarDeclarator {
            span: DUMMY_SP,
            name: Pat::Ident(signature.handle_ident),
            init: Some(Box::new(Expr::Call(CallExpr {
              span: DUMMY_SP,
              callee: ExprOrSuper::Expr(Box::new(Expr::Ident(quote_ident!(self
                .refresh_sig
                .as_str())))),
              args: vec![],
              type_args: None,
            }))),
            definite: false,
          })
          .collect(),
      }))));
    }

    // insert raw items
    for item in raw_items {
      items.push(item);
    }

    // _s(App, "useState{[count, setCount](0)}\nuseEffect{}");
    for signature in signatures {
      items.push(ModuleItem::Stmt(Stmt::Expr(ExprStmt {
        span: DUMMY_SP,
        expr: Box::new(Expr::Call(CallExpr {
          span: DUMMY_SP,
          callee: ExprOrSuper::Expr(Box::new(Expr::Ident(signature.handle_ident.clone()))),
          args: vec![
            ExprOrSpread {
              spread: None,
              expr: Box::new(Expr::Ident(signature.parent_ident.clone())),
            },
            ExprOrSpread {
              spread: None,
              expr: Box::new(Expr::Lit(Lit::Str(Str {
                span: DUMMY_SP,
                value: signature
                  .hook_calls
                  .into_iter()
                  .map(|call| call.key)
                  .collect::<Vec<String>>()
                  .join("\n")
                  .into(),
                has_escape: false,
              }))),
            },
          ],
          type_args: None,
        })),
      })));
    }

    // $RefreshReg$(_c, "App");
    // $RefreshReg$(_c2, "Foo");
    for (registration_id, fc_id) in registrations {
      items.push(ModuleItem::Stmt(Stmt::Expr(ExprStmt {
        span: DUMMY_SP,
        expr: Box::new(Expr::Call(CallExpr {
          span: DUMMY_SP,
          callee: ExprOrSuper::Expr(Box::new(Expr::Ident(quote_ident!(self
            .refresh_reg
            .as_str())))),
          args: vec![
            ExprOrSpread {
              spread: None,
              expr: Box::new(Expr::Ident(registration_id)),
            },
            ExprOrSpread {
              spread: None,
              expr: Box::new(Expr::Lit(Lit::Str(Str {
                span: DUMMY_SP,
                value: fc_id.sym.clone(),
                has_escape: false,
              }))),
            },
          ],
          type_args: None,
        })),
      })));
    }
    items
  }
}

fn is_componentish_name(name: &str) -> bool {
  name.starts_with(char::is_uppercase)
}

fn is_builtin_hook(obj: Option<Ident>, id: &str) -> bool {
  let ok = match id {
    "useState"
    | "useReducer"
    | "useEffect"
    | "useLayoutEffect"
    | "useMemo"
    | "useCallback"
    | "useRef"
    | "useContext"
    | "useImperativeHandle"
    | "useDebugValue" => true,
    _ => false,
  };
  match obj {
    Some(obj) => match obj.sym.as_ref() {
      "React" => ok,
      _ => false,
    },
    None => ok,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::import_map::{ImportHashMap, ImportMap};
  use crate::resolve::Resolver;
  use crate::swc::{EmitOptions, ParsedModule};

  use std::cell::RefCell;
  use swc_ecmascript::parser::JscTarget;

  fn tr(specifier: &str, source: &str, except: &str) -> bool {
    let module =
      ParsedModule::parse(specifier, source, JscTarget::Es2020).expect("could not parse module");
    let resolver = Rc::new(RefCell::new(Resolver::new(
      specifier,
      ImportMap::from_hashmap(ImportHashMap::default()),
      false,
      false,
    )));
    let (code, _) = module
      .transpile(resolver.clone(), &EmitOptions::default())
      .expect("could not transpile module");
    if code != except {
      println!("{}", code);
    }
    code == except
  }

  #[test]
  fn test_transpile_react_fast_refresh() {
    assert!(tr(
      "/app.jsx",
      r#"
      export default function App() {
        const [foo, setFoo] = useState(0);
        React.useEffect(() => {});
        return <h1>{foo}</h1>;
      }
      "#,
      r#"var _c;
var _s = $RefreshSig$();
export default function App() {
    _s();
    const [foo, setFoo] = useState(0);
    React.useEffect(()=>{
    });
    return React.createElement("h1", {
        __source: {
            fileName: "/app.jsx",
            lineNumber: 5
        }
    }, foo);
};
_c = App;
_s(App, "useState{[foo, setFoo](0)}\nuseEffect{}");
$RefreshReg$(_c, "App");
"#
    ));
  }
}
