//! Element Send Keys uses a session-private native input source. Null releases
//! only this command's modifiers; author action sources remain independently held.
use super::CommandError;
use oryn_native::webdriver_actions::Action;

pub fn keys(text: &[u16]) -> Result<Vec<Action>, CommandError> {
    if text.len() > 65536 {
        return Err(CommandError::invalid("element key sequence exceeds limit"));
    }
    let mut result = Vec::new();
    let mut modifiers = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let unit = text[index];
        let mut value = vec![unit];
        index += 1;
        if (0xd800..=0xdbff).contains(&unit)
            && text
                .get(index)
                .is_some_and(|unit| (0xdc00..=0xdfff).contains(unit))
        {
            value.push(text[index]);
            index += 1;
        }
        if unit == 0xe000 {
            for value in modifiers.drain(..).rev() {
                result.push(Action::Key { down: false, value });
            }
        } else if matches!(
            unit,
            0xe008 | 0xe009 | 0xe00a | 0xe03d | 0xe050 | 0xe051 | 0xe052 | 0xe053
        ) {
            if let Some(position) = modifiers.iter().position(|held| held == &value) {
                modifiers.remove(position);
                result.push(Action::Key { down: false, value });
            } else {
                modifiers.push(value.clone());
                result.push(Action::Key { down: true, value });
            }
        } else {
            result.push(Action::Key {
                down: true,
                value: value.clone(),
            });
            result.push(Action::Key { down: false, value });
        }
    }
    for value in modifiers.into_iter().rev() {
        result.push(Action::Key { down: false, value });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modifiers_toggle_and_null_releases_only_local_sequence() {
        let result = keys(&[0xe008, b'a' as u16, 0xe000, b'b' as u16]).unwrap();
        assert_eq!(
            result,
            vec![
                Action::Key {
                    down: true,
                    value: vec![0xe008]
                },
                Action::Key {
                    down: true,
                    value: vec![97]
                },
                Action::Key {
                    down: false,
                    value: vec![97]
                },
                Action::Key {
                    down: false,
                    value: vec![0xe008]
                },
                Action::Key {
                    down: true,
                    value: vec![98]
                },
                Action::Key {
                    down: false,
                    value: vec![98]
                }
            ]
        );
    }
    #[test]
    fn astral_and_lone_surrogates_are_not_lossily_decoded() {
        let result = keys(&[0xd83d, 0xde00, 0xd800]).unwrap();
        assert_eq!(result.len(), 4);
        assert_eq!(
            result[0],
            Action::Key {
                down: true,
                value: vec![0xd83d, 0xde00]
            }
        );
        assert_eq!(
            result[2],
            Action::Key {
                down: true,
                value: vec![0xd800]
            }
        );
    }
}
