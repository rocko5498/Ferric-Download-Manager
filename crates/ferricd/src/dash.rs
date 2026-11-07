use anyhow::{Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use url::Url;

#[derive(Debug, Default)]
struct Representation {
    bandwidth: u64,
    base_url: Option<String>,
    segment_urls: Vec<String>,
    template: Option<SegmentTemplate>,
}

#[derive(Debug)]
struct SegmentTemplate {
    media: String,
    start_number: u64,
    duration: Option<u64>,
}

pub async fn fetch_segments(manifest_url: &str, client: &reqwest::Client) -> Result<Vec<String>> {
    let text = client
        .get(manifest_url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_segments(manifest_url, &text)
}

pub fn parse_segments(manifest_url: &str, manifest: &str) -> Result<Vec<String>> {
    let mut reader = Reader::from_str(manifest);
    reader.config_mut().trim_text(true);

    let mut representations = Vec::new();
    let mut current = None;
    let mut in_base_url = false;
    let mut base_text = String::new();

    loop {
        match reader.read_event()? {
            Event::Start(event) if event.name().as_ref() == b"Representation" => {
                current = Some(Representation {
                    bandwidth: attr(&event, b"bandwidth")
                        .and_then(|value| value.parse::<u64>().ok())
                        .unwrap_or(0),
                    ..Representation::default()
                });
            }
            Event::End(event) if event.name().as_ref() == b"Representation" => {
                if let Some(representation) = current.take() {
                    representations.push(representation);
                }
            }
            Event::Start(event) if current.is_some() && event.name().as_ref() == b"BaseURL" => {
                in_base_url = true;
                base_text.clear();
            }
            Event::End(event) if event.name().as_ref() == b"BaseURL" => {
                if in_base_url {
                    if let Some(representation) = current.as_mut() {
                        representation.base_url = Some(base_text.trim().to_string());
                    }
                    in_base_url = false;
                }
            }
            Event::Text(text) if in_base_url => {
                base_text.push_str(&text.decode()?);
            }
            Event::Empty(event) if current.is_some() && event.name().as_ref() == b"SegmentURL" => {
                if let Some(media) = attr(&event, b"media") {
                    if let Some(representation) = current.as_mut() {
                        representation.segment_urls.push(media);
                    }
                }
            }
            Event::Empty(event)
                if current.is_some() && event.name().as_ref() == b"SegmentTemplate" =>
            {
                if let Some(media) = attr(&event, b"media") {
                    if let Some(representation) = current.as_mut() {
                        representation.template = Some(SegmentTemplate {
                            media,
                            start_number: attr(&event, b"startNumber")
                                .and_then(|value| value.parse::<u64>().ok())
                                .unwrap_or(1),
                            duration: attr(&event, b"duration")
                                .and_then(|value| value.parse::<u64>().ok()),
                        });
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let representation = representations
        .iter()
        .max_by_key(|representation| representation.bandwidth)
        .ok_or_else(|| anyhow::anyhow!("DASH MPD has no representations"))?;
    let base = representation.base_url.as_deref().unwrap_or("");
    let segment_urls = if !representation.segment_urls.is_empty() {
        representation.segment_urls.clone()
    } else if let Some(template) = &representation.template {
        expand_template(template)?
    } else {
        return Err(anyhow::anyhow!(
            "DASH representation has no SegmentURL or supported SegmentTemplate"
        ));
    };

    segment_urls
        .iter()
        .map(|segment| resolve_url(manifest_url, base, segment))
        .collect()
}

fn expand_template(template: &SegmentTemplate) -> Result<Vec<String>> {
    let Some(duration) = template.duration else {
        return Err(anyhow::anyhow!(
            "DASH SegmentTemplate without SegmentTimeline needs duration"
        ));
    };
    if duration == 0 {
        return Err(anyhow::anyhow!("DASH SegmentTemplate duration is zero"));
    }

    Ok((template.start_number..template.start_number + 32)
        .map(|number| {
            template
                .media
                .replace("$Number$", &number.to_string())
                .replace("$Number%05d$", &format!("{number:05}"))
        })
        .collect())
}

fn resolve_url(manifest_url: &str, base: &str, segment: &str) -> Result<String> {
    let manifest =
        Url::parse(manifest_url).with_context(|| format!("parse MPD URL {manifest_url}"))?;
    let base = if base.is_empty() {
        manifest
    } else {
        manifest
            .join(base)
            .with_context(|| format!("resolve DASH BaseURL {base}"))?
    };
    Ok(base
        .join(segment)
        .with_context(|| format!("resolve DASH segment URL {segment}"))?
        .to_string())
}

fn attr(event: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    event.attributes().flatten().find_map(|attr| {
        (attr.key.as_ref() == name)
            .then(|| {
                attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(std::borrow::Cow::into_owned)
            })
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dash_segment_list() -> Result<()> {
        let manifest = r#"
            <MPD>
              <Period>
                <AdaptationSet>
                  <Representation bandwidth="2000">
                    <BaseURL>video/</BaseURL>
                    <SegmentList>
                      <SegmentURL media="seg-1.m4s" />
                      <SegmentURL media="seg-2.m4s" />
                    </SegmentList>
                  </Representation>
                </AdaptationSet>
              </Period>
            </MPD>
        "#;

        assert_eq!(
            parse_segments("https://example.com/live/manifest.mpd", manifest)?,
            vec![
                "https://example.com/live/video/seg-1.m4s".to_string(),
                "https://example.com/live/video/seg-2.m4s".to_string(),
            ]
        );
        Ok(())
    }
}
