# Bank 3: English accents and dialects
#
# Status: partly spent. Its sign-off misses (", ta", ", sweet as", ", lah") and
# eight misreads (month order, put-together merges, "make Documents a new folder",
# "make a zip archive from") chose rules in src/parse/rewrite.rs; lines that then
# passed moved to src/parse/corpus/13-bank-3.txt. The rest are unused.
# Commands as people with different English accents and dialects would type or
# dictate them: regional vocabulary and grammar, plus the words a dictation app
# plausibly mishears in that accent. Each variant means exactly what its seed
# means and carries the seed's expected intent.
# Run: `HELDOUT=bank-3 cargo test --lib heldout -- --ignored --nocapture`.
#
# Seeds (command => expected intent). Every line in this bank uses one of these.
#
# seed: move all PDFs from Downloads to Documents => {"intent":"move","src":{"kind":"set","folder":"Downloads","filter":{"type":"pdfs"}},"to":"Documents"}
# seed: move screenshots from today to Screenshots => {"intent":"move","src":{"kind":"set","filter":{"type":"screenshots","day":"today"}},"to":"Screenshots"}
# seed: move report.pdf to Documents => {"intent":"move","src":{"kind":"item","name":"report.pdf"},"to":"Documents"}
# seed: move all videos from Desktop to Movies => {"intent":"move","src":{"kind":"set","folder":"Desktop","filter":{"type":"videos"}},"to":"Movies"}
# seed: move files from Desktop to Documents => {"intent":"move","src":{"kind":"set","folder":"Desktop","filter":{"type":"files"}},"to":"Documents"}
# seed: copy all images from Pictures to Backup => {"intent":"copy","src":{"kind":"set","folder":"Pictures","filter":{"type":"images"}},"to":"Backup"}
# seed: copy report.pdf to Desktop => {"intent":"copy","src":{"kind":"item","name":"report.pdf"},"to":"Desktop"}
# seed: delete screenshots older than 30 days => {"intent":"trash","src":{"kind":"set","filter":{"type":"screenshots","olderThanDays":30}}}
# seed: clean up Downloads older than 90 days => {"intent":"trash","src":{"kind":"set","folder":"Downloads","filter":{"type":"files","olderThanDays":90}}}
# seed: move installers from Downloads to Trash => {"intent":"trash","src":{"kind":"set","folder":"Downloads","filter":{"type":"installers"}}}
# seed: delete dmg files from Downloads => {"intent":"trash","src":{"kind":"set","folder":"Downloads","filter":{"type":"installers"}}}
# seed: delete report.pdf => {"intent":"trash","src":{"kind":"item","name":"report.pdf"}}
# seed: delete everything in Downloads => {"intent":"trash","src":{"kind":"set","folder":"Downloads","filter":{"type":"files"}}}
# seed: delete files larger than 1 GB in Downloads => {"intent":"trash","src":{"kind":"set","folder":"Downloads","filter":{"type":"files","largerThanMb":1024.0}}}
# seed: delete files named untitled on Desktop => {"intent":"trash","src":{"kind":"set","folder":"Desktop","filter":{"type":"files","nameContains":"untitled"}}}
# seed: list files in Downloads larger than 500 MB => {"intent":"list","src":{"kind":"set","folder":"Downloads","filter":{"type":"files","largerThanMb":500.0}}}
# seed: list files in Downloads => {"intent":"list","src":{"kind":"set","folder":"Downloads","filter":{"type":"files"}}}
# seed: list PDFs in Downloads => {"intent":"list","src":{"kind":"set","folder":"Downloads","filter":{"type":"pdfs"}}}
# seed: list screenshots from today => {"intent":"list","src":{"kind":"set","filter":{"type":"screenshots","day":"today"}}}
# seed: list files in Downloads older than 1 year => {"intent":"list","src":{"kind":"set","folder":"Downloads","filter":{"type":"files","olderThanDays":365}}}
# seed: list videos in Movies => {"intent":"list","src":{"kind":"set","folder":"Movies","filter":{"type":"videos"}}}
# seed: list files in Documents from this week => {"intent":"list","src":{"kind":"set","folder":"Documents","filter":{"type":"files","since":"2026-10-26"}}}
# seed: count PDFs in Documents => {"intent":"count","src":{"kind":"set","folder":"Documents","filter":{"type":"pdfs"}}}
# seed: count files in Downloads => {"intent":"count","src":{"kind":"set","folder":"Downloads","filter":{"type":"files"}}}
# seed: show size of Downloads => {"intent":"size","folder":"Downloads"}
# seed: list duplicate files in Downloads => {"intent":"duplicates","folder":"Downloads"}
# seed: open Downloads in Finder => {"intent":"reveal","target":"Downloads"}
# seed: undo last => {"intent":"undo"}
# seed: show recent actions => {"intent":"recent_actions"}
# seed: rename report.pdf to Q3-report.pdf => {"intent":"rename","src":{"kind":"item","name":"report.pdf"},"new_name":"Q3-report.pdf"}
# seed: replace spaces with dashes in files in Downloads => {"intent":"replace_in_names","src":{"kind":"set","folder":"Downloads","filter":{"type":"files"}},"find":" ","replace":"-"}
# seed: lowercase file names in Downloads => {"intent":"lowercase","src":{"kind":"set","folder":"Downloads","filter":{"type":"files"}}}
# seed: add date prefix to files in Screenshots => {"intent":"add_date_prefix","src":{"kind":"set","folder":"Screenshots","filter":{"type":"files"}}}
# seed: number images in Trip as trip-001 => {"intent":"number","src":{"kind":"set","folder":"Trip","filter":{"type":"images"}},"base":"trip-","width":3}
# seed: add prefix Invoice- to files named bill in Downloads => {"intent":"add_prefix","src":{"kind":"set","folder":"Downloads","filter":{"type":"files","nameContains":"bill"}},"prefix":"Invoice-"}
# seed: create folders 2026, 2027 in Documents => {"intent":"create_folders","names":["2026","2027"],"parent":"Documents"}
# seed: create a folder named Invoices in Documents => {"intent":"create_folders","names":["Invoices"],"parent":"Documents"}
# seed: sort Downloads by type => {"intent":"sort_by_type","folder":"Downloads"}
# seed: sort Screenshots by month => {"intent":"sort_by_month","folder":"Screenshots"}
# seed: convert photo.heic to png => {"intent":"convert_image","src":{"kind":"item","name":"photo.heic"},"format":"png"}
# seed: convert all HEIC in Pictures to jpg => {"intent":"convert_image","src":{"kind":"set","folder":"Pictures","filter":{"type":"files","ext":["heic","heif"]}},"format":"jpg"}
# seed: resize all images in Trip to 1600 pixels wide => {"intent":"resize_image","src":{"kind":"set","folder":"Trip","filter":{"type":"images"}},"width":1600}
# seed: merge all PDFs in Downloads into combined.pdf => {"intent":"merge_pdf","src":{"kind":"set","folder":"Downloads","filter":{"type":"pdfs"}},"output":"combined.pdf"}
# seed: split report.pdf into pages => {"intent":"split_pdf","src":{"kind":"item","name":"report.pdf"}}
# seed: extract pages 2 to 5 from report.pdf into excerpt.pdf => {"intent":"extract_pages","src":{"kind":"item","name":"report.pdf"},"from":2,"to":5,"output":"excerpt.pdf"}
# seed: zip Project into Project.zip => {"intent":"zip","src":{"kind":"item","name":"Project"},"output":"Project.zip"}
# seed: unzip archive.zip => {"intent":"unzip","src":{"kind":"item","name":"archive.zip"}}
