import { mount } from "svelte";
import "./styles/app.css";
import Reader from "./components/Reader.svelte";

mount(Reader, { target: document.getElementById("app") });
