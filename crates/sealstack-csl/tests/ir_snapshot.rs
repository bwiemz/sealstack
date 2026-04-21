use sealstack_csl::codegen::policy;
use sealstack_csl::{parser, types};

#[test]
fn simple_policy_ir_snapshot() {
    let src = r#"
        schema Doc {
            id:    Ulid   @primary
            owner: String

            policy {
                read: caller.id == self.owner
            }
        }
    "#;
    let file = parser::parse_file(src).unwrap();
    let typed = types::check(&file).unwrap();
    let ir = policy::lower_schema_to_ir(&typed, "Doc").expect("lower");
    let hex: String = ir
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    insta::assert_snapshot!("doc_read_ir", hex);
}
