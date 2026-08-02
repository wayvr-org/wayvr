import { exit } from "process";
import * as fsp from "fs/promises";
import path from "path";
import * as fs from "fs";

const base_url = process.env["LLAMA_BASE_URL"] as string;
const model_name = process.env["MODEL"] as string;
const template_name = process.env["TEMPLATE"] as string;
let lang_path = process.env["LANG_PATH"] as string;

if (model_name === undefined) {
	console.log("MODEL not set");
	exit(-1);
}

if (template_name === undefined) {
	console.log("TEMPLATE not set");
	exit(-1);
}

if (lang_path === undefined) {
	console.log(
		`LANG_PATH is not set.
Try one of these:
LANG_PATH=../../uidev/assets/lang/ ./run.sh
LANG_PATH=../../dash-frontend/assets/lang/ ./run.sh
LANG_PATH=../../wayvr/src/assets/lang/ ./run.sh`,
	);
	exit(-1);
}

lang_path = path.resolve(__dirname + "/" + lang_path);
if (lang_path === undefined || !fs.existsSync(lang_path)) {
	console.log("Invalid or non-existent LANG_PATH");
	exit(-1);
}

const current_path = path.resolve(__dirname);
const templates_path = path.resolve(__dirname + "/templates");

async function loop_object(
	obj: any,
	initial_str: string,
	callback: (key: string, value: string) => Promise<void>,
) {
	for (var key in obj) {
		let full_key = initial_str + key;
		if (typeof obj[key] === "object" && obj[key] !== null) {
			await loop_object(obj[key], full_key + ".", callback);
		} else if (obj.hasOwnProperty(key)) {
			await callback(full_key, obj[key]);
		}
	}
}

function extract_backticks(str: string) {
	const regex = /`([^`]+)`/g;
	return str.match(regex)?.map((match) => match.slice(1, -1).trim());
}

function set_i18n_key(obj: any, key: string, value: string | undefined) {
	const parts = key.split(".");
	let cur_level = obj;
	for (let i = 0; i < parts.length - 1; i++) {
		const part = parts[i]!;
		if (!cur_level[part]) {
			cur_level[part] = {};
		}
		cur_level = cur_level[part];
	}
	cur_level[parts[parts.length - 1]!] = value;
}

function key_exists(obj: any, key: string) {
	const parts = key.split(".");
	let level = obj;

	for (let i = 0; i < parts.length; i++) {
		const part = parts[i]!;
		if (!level || !level[part]) {
			return false;
		}
		level = level[part];
	}

	return true;
}

interface Example {
	key: string;
	en: string;
	translated: string;
}

interface Template {
	full_name: string; // "Polish"
	examples: Example[];
}

interface CsvEntry {
	key: string;
	english: string;
	context: string;
}

function parse_csv_line(line: string): [string, string, string] {
	const result: string[] = [];
	let current = "";
	let inQuotes = false;
	for (let i = 0; i < line.length; i++) {
		const ch = line[i]!;
		if (ch === '"') {
			inQuotes = !inQuotes;
		} else if (ch === "," && !inQuotes) {
			result.push(current.trim());
			current = "";
		} else {
			current += ch;
		}
	}
	result.push(current.trim());
	return [result[0]!, result[1]!, result[2] ?? ""];
}

function parse_csv(text: string): CsvEntry[] {
	const lines: string[] = [];
	let current = "";
	let inQuotes = false;
	for (const line of text.split("\n")) {
		const wasInQuotes = inQuotes;
		for (const ch of line) {
			if (ch === '"') {
				inQuotes = !inQuotes;
			}
		}
		if (wasInQuotes) {
			current += line + "\n";
		} else {
			if (current) lines.push(current);
			current = line + "\n";
		}
	}
	if (current) lines.push(current);
	return lines.filter((l) => l.trim()).map(parse_csv_line).map(([key, english, context]) => ({
		key,
		english,
		context,
	}));
}

function csv_to_json(entries: CsvEntry[]): any {
	const result: any = {};
	for (const entry of entries) {
		set_i18n_key(result, entry.key, entry.english);
	}
	return result;
}

function get_nested_value(obj: any, key: string): string | undefined {
	const parts = key.split(".");
	let level = obj;
	for (const part of parts) {
		if (!level || typeof level !== "object") {
			return undefined;
		}
		level = level[part];
	}
	return level;
}

function resolve_variables(
	text: string,
	translated_json: any,
	all_entries: CsvEntry[],
): string {
	return text.replace(/\$\{([^}]+)\}/g, (_, key) => {
		const val = get_nested_value(translated_json, key);
		if (val !== undefined) {
			return val;
		}
		const entry = all_entries.find((e) => e.key === key);
		if (entry !== undefined) {
			return entry.english;
		}
		return `${key}`;
	});
}

function gen_prompt(
	description: string,
	template: Template,
	key: string,
	english_translation: string,
	context: string,
) {
	let num = 1;
	for (const example of template.examples) {
		description += "\nExample " + num + ":\n\n";
		description +=
			"Translate key `" +
			example.key +
			"` from English to " +
			template.full_name +
			":\n\n";
		description += "```\n";
		description += example.en + "\n";
		description += "```\n\n";
		description += "Result:\n\n";
		description += "```\n";
		description += example.translated + "\n";
		description += "```\n";
		num += 1;
	}
	description += "\nEnd of examples.\n\n";
	description +=
		"Translate key `" +
		key +
		"` from English to " +
		template.full_name +
		":\n\n";
	description += "Context: " + context + "\n\n";
	description += "Style: These are UI elements in a software, so keep the translations cocise and with an appropriate tone. Don't use polite/formal language. Make sure it sounds natural.\n\n";
	description += "```\n";
	description += english_translation + "\n";
	description += "```\n";
	return description;
}

async function run() {
	const template = JSON.parse(
		await fsp.readFile(templates_path + "/" + template_name + ".json", "utf-8"),
	) as Template;

	let description_txt = await fsp.readFile(
		current_path + "/description.txt",
		"utf-8",
	);
	description_txt = description_txt.replaceAll(
		"{TARGET_LANG}",
		template.full_name,
	);

	const csv_text = (await fsp.readFile(lang_path + "/en.csv", "utf-8")) as any;
	const csv_entries = parse_csv(csv_text);
	const orig_english_json = csv_to_json(csv_entries);

	let llm_translated_json: any = {};
	const translated_json_path = lang_path + "/" + template_name + ".json";
	if (await fsp.exists(translated_json_path)) {
		console.log("Loading file", translated_json_path);
		llm_translated_json = JSON.parse(
			(await fsp.readFile(translated_json_path)).toString(),
		);
	}

	if (template_name === "en") {
		await fsp.writeFile(
			lang_path + "/en.json",
			JSON.stringify(orig_english_json, undefined, "\t"),
		);
		console.log("Translation en finished");
		return;
	}

	let orig_translated_json = {};
	try {
		orig_translated_json = JSON.parse(
			(
				await fsp.readFile(lang_path + "/" + template_name + ".json")
			).toString(),
		);
	} catch (_e) {}

	let total_count = 0;
	for (const _entry of csv_entries) {
		total_count += 1;
	}

	await loop_object(llm_translated_json, "", async (key, _) => {
		if (!key_exists(orig_english_json, key)) {
			console.log("Removing key", key);
			set_i18n_key(llm_translated_json, key, undefined);
			await fsp.writeFile(
				translated_json_path,
				JSON.stringify(llm_translated_json, undefined, "\t"),
			);
		}
	});

	for (const entry of csv_entries) {
		const key = entry.key;
		const english_translation = entry.english;
		const context = entry.context;

		if (key_exists(orig_translated_json, key)) {
			continue;
		}

		if (key_exists(llm_translated_json, key)) {
			continue;
		}

		console.log("Translating", key, "...");

		const resolved_context = resolve_variables(context, llm_translated_json, csv_entries);

		const prompt = gen_prompt(
			description_txt,
			template,
			key,
			english_translation,
			resolved_context,
		);

		const response = await fetch(base_url + "/v1/chat/completions", {
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify({
				model: model_name,
				messages: [{ role: "user", content: prompt }],
				chat_template_kwargs: {
					enable_thinking: false,
				},
				seed: 12345,
			}),
		});
		if (!response.ok) {
			const errorBody = await response.text();
			throw new Error("API error " + response.status + ": " + errorBody);
		}
		const json = (await response.json()) as any;

		if (!json.choices || !json.choices[0] || !json.choices[0].message) {
			console.log("Full response:", JSON.stringify(json, null, "\t"));
			throw new Error("Unexpected response format: " + JSON.stringify(json));
		}

		const msg = extract_backticks(json.choices[0].message.content);
		if (msg === undefined || msg[0] === undefined) {
			throw new Error(
				"backticks failed. Raw content: " + json.choices[0].message.content,
			);
		}

		console.log(" >>>", msg);

		set_i18n_key(llm_translated_json, key, msg[0]);
		await fsp.writeFile(
			translated_json_path,
			JSON.stringify(llm_translated_json, undefined, "\t"),
		);
	}

	console.log("Translation", template_name, "finished");
}

run().catch((e) => {
	console.log("Fatal error:", e);
	exit(-1);
});
