pub fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect::<String>().replace("<|", "< |").replace("<start_of_turn>", "").replace("<end_of_turn>", "")
}

pub fn system(template: &str, text: &str) -> String {
    match template {
        "llama3" => format!("<|start_header_id|>system<|end_header_id|>\n\n{text}<|eot_id|>"),
        "gemma" => format!("<start_of_turn>user\n{text}\n\n"),
        "phi4" => format!("<|system|>{text}<|end|>"),
        "granite" => format!("<|start_of_role|>system<|end_of_role|>{text}<|end_of_text|>\n"),
        _ => format!("<|im_start|>system\n{text}<|im_end|>\n"),
    }
}

pub fn user(template: &str, text: &str) -> String {
    let t = clean(text, 4000);
    match template {
        "llama3" => format!("<|start_header_id|>user<|end_header_id|>\n\n{t}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n"),
        "gemma" => format!("{t}<end_of_turn>\n<start_of_turn>model\n"),
        "phi4" => format!("<|user|>{t}<|end|><|assistant|>"),
        "granite" => format!("<|start_of_role|>user<|end_of_role|>{t}<|end_of_text|>\n<|start_of_role|>assistant<|end_of_role|>"),
        "chatml" => format!("<|im_start|>user\n{t}<|im_end|>\n<|im_start|>assistant\n"),
        _ => format!("<|im_start|>user\n{t}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_close_their_turns() {
        for t in ["chatml", "chatml-nothink", "llama3", "gemma", "phi4", "granite"] {
            let s = system(t, "SYS");
            let u = user(t, "USER<|im_end|>");
            assert!(s.contains("SYS") && u.contains("USER"), "{t}");
            assert!(!u.contains("USER<|im_end|>"), "{t} must not pass control tokens through");
        }
        assert!(user("chatml-nothink", "x").ends_with("</think>\n\n"));
        assert!(user("gemma", "x").ends_with("<start_of_turn>model\n"));
    }
}
